use std::{collections::HashSet, panic::AssertUnwindSafe, sync::Arc};

use futures::{FutureExt, StreamExt};
use rig::{
    agent::{
        AgentHook, HookContext, MultiTurnStreamItem, ObservationAction, TextDelta, ToolCall,
        ToolCallAction, ToolResultAction, ToolResultEvent,
    },
    streaming::StreamingPrompt,
};
use rust_agent::{build_agent, config::Settings, rag::Rag};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum Event {
    Ready(String),
    Text { turn: usize, text: String },
    ToolStarted { id: String, args: String },
    ToolFinished { id: String },
    Retried(usize),
    Finished { turn: usize, output: String },
    Failed(String),
    Unavailable(String),
}

struct ToolHook(mpsc::Sender<Event>);

impl AgentHook for ToolHook {
    async fn on_text_delta(&self, cx: &HookContext, event: TextDelta<'_>) -> ObservationAction {
        if self
            .0
            .send(Event::Text {
                turn: cx.turn(),
                text: event.delta.into(),
            })
            .await
            .is_err()
        {
            return ObservationAction::Stop("Chat closed".into());
        }
        ObservationAction::Continue
    }

    async fn on_tool_call(&self, _: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        let event = Event::ToolStarted {
            id: event.internal_call_id.into(),
            args: event.args.into(),
        };
        if self.0.send(event).await.is_err() {
            return ToolCallAction::stop("Chat closed");
        }
        ToolCallAction::run()
    }

    async fn on_tool_result(
        &self,
        _: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        let event = Event::ToolFinished {
            id: event.internal_call_id.into(),
        };
        if self.0.send(event).await.is_err() {
            return ToolResultAction::stop("Chat closed");
        }
        ToolResultAction::keep()
    }
}

/// The worker owns one agent and conversation for the lifetime of the application.
pub async fn run(mut requests: mpsc::Receiver<String>, events: mpsc::Sender<Event>) {
    let initialized = AssertUnwindSafe(async {
        let settings = Settings::build()?;
        let model = settings
            .agents
            .get("alpha")
            .ok_or_else(|| anyhow::anyhow!("Missing agents.alpha configuration"))?
            .model
            .clone();
        let rag = Arc::new(Rag::build(settings.knowledge).await?);
        anyhow::Ok((build_agent(&settings.agents, rag)?, model))
    })
    .catch_unwind()
    .await;
    let (agent, model) = match initialized {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            let _ = events.send(Event::Unavailable(format!("{error:#}"))).await;
            return;
        }
        Err(_) => {
            let _ = events
                .send(Event::Unavailable("Agent initialization panicked".into()))
                .await;
            return;
        }
    };
    if events.send(Event::Ready(model)).await.is_err() {
        return;
    }
    let conversation = uuid::Uuid::now_v7().to_string();
    while let Some(message) = requests.recv().await {
        let result = AssertUnwindSafe(async {
            let mut stream = agent
                .stream_prompt(message)
                .conversation(&conversation)
                .max_turns(50)
                .add_hook(ToolHook(events.clone()))
                .await;
            let mut final_response = None;
            while let Some(item) = stream.next().await {
                match item? {
                    MultiTurnStreamItem::ModelTurnRetried { turn } => {
                        events.send(Event::Retried(turn)).await?;
                    }
                    MultiTurnStreamItem::FinalResponse(response) => final_response = Some(response),
                    _ => {}
                }
            }
            let response = final_response
                .ok_or_else(|| anyhow::anyhow!("Stream ended without a final response"))?;
            events
                .send(Event::Finished {
                    turn: response.completion_calls.len(),
                    output: response.output,
                })
                .await?;
            anyhow::Ok(())
        })
        .catch_unwind()
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if events
                    .send(Event::Failed(format!("{error:#}")))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(_) => {
                let _ = events
                    .send(Event::Unavailable(
                        "Agent task panicked. Quit and reopen the app to reconnect.".into(),
                    ))
                    .await;
                break;
            }
        }
    }
}

#[derive(Debug)]
pub enum Entry {
    User(String),
    Assistant {
        turn: usize,
        text: String,
    },
    ToolIntent {
        id: String,
        intent: String,
        running: bool,
    },
}

#[derive(Default)]
pub struct Conversation {
    pub entries: Vec<Entry>,
    pub busy: bool,
    pub ready: bool,
    pub model: String,
    pub error: Option<String>,
    streaming_turn: Option<usize>,
    running_tools: HashSet<String>,
    request_start: usize,
}

impl Conversation {
    pub fn start(&mut self, message: String) {
        self.entries.push(Entry::User(message));
        self.request_start = self.entries.len();
        self.busy = true;
        self.error = None;
        self.streaming_turn = None;
        self.running_tools.clear();
    }

    pub fn show_thinking(&self) -> bool {
        self.busy && self.streaming_turn.is_none() && self.running_tools.is_empty()
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Ready(model) => {
                self.ready = true;
                self.model = model;
            }
            Event::Text { turn, text } => {
                if text.is_empty() {
                    return;
                }
                self.streaming_turn = Some(turn);
                if let Some(Entry::Assistant { text: current, .. }) = self.entries
                    [self.request_start..]
                    .iter_mut()
                    .find(|entry| matches!(entry, Entry::Assistant { turn: t, .. } if *t == turn))
                {
                    current.push_str(&text);
                } else {
                    self.entries.push(Entry::Assistant { turn, text });
                }
            }
            Event::ToolStarted { id, args } => {
                self.streaming_turn = None;
                self.running_tools.insert(id.clone());
                if let Ok(args) = serde_json::from_str::<serde_json::Value>(&args)
                    && let Some(intent) = args.get("intent").and_then(|value| value.as_str())
                    && !intent.trim().is_empty()
                {
                    self.entries.push(Entry::ToolIntent {
                        id,
                        intent: intent.to_owned(),
                        running: true,
                    });
                }
            }
            Event::ToolFinished { id } => {
                self.running_tools.remove(&id);
                if let Some(Entry::ToolIntent { running, .. }) =
                    self.entries[self.request_start..].iter_mut().find(
                        |entry| matches!(entry, Entry::ToolIntent { id: call, .. } if call == &id),
                    )
                {
                    *running = false;
                }
            }
            Event::Retried(turn) => {
                if self.streaming_turn == Some(turn) {
                    self.streaming_turn = None;
                }
                let start = self.request_start;
                let mut index = 0;
                self.entries.retain(|entry| {
                    let keep = index < start
                        || !matches!(entry, Entry::Assistant { turn: t, .. } if *t == turn);
                    index += 1;
                    keep
                });
            }
            Event::Finished { turn, output } => {
                // FinalResponse repeats the last turn; replace its deltas rather than appending.
                if let Some(Entry::Assistant { text, .. }) = self.entries[self.request_start..]
                    .iter_mut()
                    .find(|entry| matches!(entry, Entry::Assistant { turn: t, .. } if *t == turn))
                {
                    *text = output;
                } else if !output.is_empty() {
                    self.entries.push(Entry::Assistant { turn, text: output });
                }
                self.finish();
            }
            Event::Failed(error) => {
                self.error = Some(error);
                self.finish();
            }
            Event::Unavailable(error) => {
                self.ready = false;
                self.error = Some(error);
                self.finish();
            }
        }
    }

    fn finish(&mut self) {
        self.busy = false;
        self.streaming_turn = None;
        self.running_tools.clear();
        for entry in &mut self.entries[self.request_start..] {
            if let Entry::ToolIntent { running, .. } = entry {
                *running = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_finalization_retries_and_followups_do_not_duplicate_text() {
        let mut chat = Conversation::default();
        chat.start("Hello".into());
        chat.apply(Event::Text {
            turn: 1,
            text: "wrong".into(),
        });
        chat.apply(Event::Retried(1));
        chat.apply(Event::Text {
            turn: 2,
            text: "Hel".into(),
        });
        chat.apply(Event::Text {
            turn: 2,
            text: "lo".into(),
        });
        chat.apply(Event::Finished {
            turn: 2,
            output: "Hello!".into(),
        });
        assert_eq!(chat.entries.len(), 2);
        assert!(matches!(&chat.entries[1], Entry::Assistant { text, .. } if text == "Hello!"));
        chat.start("Again".into());
        chat.apply(Event::Finished {
            turn: 1,
            output: "Welcome back".into(),
        });
        assert_eq!(chat.entries.len(), 4);
        assert!(matches!(&chat.entries[1], Entry::Assistant { text, .. } if text == "Hello!"));
        assert!(!chat.busy);
    }

    #[test]
    fn tool_intents_keep_separate_rows_and_finish_by_id() {
        let mut chat = Conversation::default();
        chat.start("Use tools".into());
        assert!(chat.show_thinking());
        for args in ["{}", "not json", r#"{"intent":42}"#, r#"{"intent":"  "}"#] {
            chat.apply(Event::ToolStarted {
                id: "no-intent".into(),
                args: args.into(),
            });
            assert_eq!(chat.entries.len(), 1);
            assert!(!chat.show_thinking());
            chat.apply(Event::ToolFinished {
                id: "no-intent".into(),
            });
            assert!(chat.show_thinking());
        }
        chat.apply(Event::ToolStarted {
            id: "a".into(),
            args: r#"{"intent":"Fetching documentation"}"#.into(),
        });
        chat.apply(Event::ToolStarted {
            id: "b".into(),
            args: r#"{"intent":"Generating a UUID"}"#.into(),
        });
        assert!(!chat.show_thinking());
        assert!(
            matches!(&chat.entries[1], Entry::ToolIntent { intent, running: true, .. } if intent == "Fetching documentation")
        );
        assert!(
            matches!(&chat.entries[2], Entry::ToolIntent { intent, running: true, .. } if intent == "Generating a UUID")
        );
        chat.apply(Event::ToolFinished { id: "b".into() });
        assert!(!chat.show_thinking());
        chat.apply(Event::ToolFinished { id: "b".into() });
        assert!(!chat.show_thinking());
        assert!(matches!(
            &chat.entries[1],
            Entry::ToolIntent { running: true, .. }
        ));
        assert!(matches!(
            &chat.entries[2],
            Entry::ToolIntent { running: false, .. }
        ));
        chat.apply(Event::ToolFinished { id: "a".into() });
        assert!(matches!(
            &chat.entries[1],
            Entry::ToolIntent { running: false, .. }
        ));
        assert!(chat.show_thinking());
        assert_eq!(chat.entries.len(), 3);
    }

    #[test]
    fn ending_a_request_stops_intent_spinners_and_preserves_rows() {
        for event in [
            Event::Finished {
                turn: 1,
                output: "Done".into(),
            },
            Event::Failed("connection lost".into()),
            Event::Unavailable("worker stopped".into()),
        ] {
            let mut chat = Conversation::default();
            chat.start("Use tools".into());
            chat.apply(Event::ToolStarted {
                id: "a".into(),
                args: r#"{"intent":"Fetching documentation"}"#.into(),
            });
            chat.apply(event);
            assert!(!chat.busy);
            assert!(!chat.show_thinking());
            assert!(matches!(
                &chat.entries[1],
                Entry::ToolIntent { running: false, .. }
            ));
            chat.start("Again".into());
            assert!(chat.show_thinking());
            assert!(matches!(
                &chat.entries[1],
                Entry::ToolIntent { running: false, .. }
            ));
        }
    }

    #[test]
    fn thinking_returns_after_retries_or_the_last_tool_until_streaming() {
        let mut chat = Conversation::default();
        chat.start("Hello".into());
        chat.apply(Event::Text {
            turn: 1,
            text: String::new(),
        });
        assert!(chat.show_thinking());
        chat.apply(Event::Text {
            turn: 1,
            text: "Searching".into(),
        });
        assert!(!chat.show_thinking());
        chat.apply(Event::Retried(1));
        assert!(chat.show_thinking());
        chat.apply(Event::Text {
            turn: 2,
            text: "Searching again".into(),
        });
        chat.apply(Event::ToolStarted {
            id: "a".into(),
            args: r#"{"intent":"Fetching documentation"}"#.into(),
        });
        assert!(!chat.show_thinking());
        chat.apply(Event::ToolFinished { id: "a".into() });
        assert!(chat.show_thinking());
        chat.apply(Event::Text {
            turn: 3,
            text: "Here is the answer".into(),
        });
        assert!(!chat.show_thinking());
        assert!(chat.busy);
        assert!(matches!(
            &chat.entries[2],
            Entry::ToolIntent { running: false, .. }
        ));
        assert!(matches!(&chat.entries[3], Entry::Assistant { turn: 3, .. }));
    }
}
