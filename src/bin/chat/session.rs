use std::{panic::AssertUnwindSafe, sync::Arc};

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
    Text {
        turn: usize,
        text: String,
    },
    ToolStarted {
        id: String,
        name: String,
        args: String,
    },
    ToolFinished {
        id: String,
        output: String,
        failed: bool,
    },
    Retried(usize),
    Finished {
        turn: usize,
        output: String,
    },
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
            name: event.tool_name.into(),
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
        let output = event
            .raw_result
            .error()
            .map(|error| error.to_string())
            .unwrap_or_else(|| event.presentation.render());
        let event = Event::ToolFinished {
            id: event.internal_call_id.into(),
            output,
            failed: event.raw_result.is_error(),
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

#[derive(Debug, PartialEq)]
pub enum ToolStatus {
    Running,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug)]
pub enum Entry {
    User(String),
    Assistant {
        turn: usize,
        text: String,
    },
    Tool {
        id: String,
        name: String,
        args: String,
        output: String,
        status: ToolStatus,
        expanded: bool,
    },
}

#[derive(Default)]
pub struct Conversation {
    pub entries: Vec<Entry>,
    pub busy: bool,
    pub ready: bool,
    pub model: String,
    pub error: Option<String>,
    request_start: usize,
}

impl Conversation {
    pub fn start(&mut self, message: String) {
        self.entries.push(Entry::User(message));
        self.request_start = self.entries.len();
        self.busy = true;
        self.error = None;
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Ready(model) => {
                self.ready = true;
                self.model = model;
            }
            Event::Text { turn, text } => {
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
            Event::ToolStarted { id, name, args } => self.entries.push(Entry::Tool {
                id,
                name,
                args,
                output: String::new(),
                status: ToolStatus::Running,
                expanded: false,
            }),
            Event::ToolFinished { id, output, failed } => {
                if let Some(Entry::Tool {
                    output: result,
                    status,
                    ..
                }) = self.entries[self.request_start..]
                    .iter_mut()
                    .find(|entry| matches!(entry, Entry::Tool { id: call, .. } if call == &id))
                {
                    *result = output;
                    *status = if failed {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Completed
                    };
                }
            }
            Event::Retried(turn) => {
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
        for entry in &mut self.entries[self.request_start..] {
            if let Entry::Tool { status, .. } = entry
                && *status == ToolStatus::Running
            {
                *status = ToolStatus::Interrupted;
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
    fn repeated_tools_match_by_id_and_unfinished_tools_become_interrupted() {
        let mut chat = Conversation::default();
        chat.start("Use tools".into());
        for id in ["a", "b", "c"] {
            chat.apply(Event::ToolStarted {
                id: id.into(),
                name: "Beta".into(),
                args: "{}".into(),
            });
        }
        chat.apply(Event::ToolFinished {
            id: "b".into(),
            output: "second".into(),
            failed: false,
        });
        chat.apply(Event::ToolFinished {
            id: "a".into(),
            output: "error".into(),
            failed: true,
        });
        chat.apply(Event::Failed("connection lost".into()));
        assert!(matches!(
            &chat.entries[1],
            Entry::Tool {
                status: ToolStatus::Failed,
                ..
            }
        ));
        assert!(
            matches!(&chat.entries[2], Entry::Tool { output, status: ToolStatus::Completed, .. } if output == "second")
        );
        assert!(matches!(
            &chat.entries[3],
            Entry::Tool {
                status: ToolStatus::Interrupted,
                ..
            }
        ));
        assert!(!chat.busy);
        assert_eq!(chat.error.as_deref(), Some("connection lost"));
    }
}
