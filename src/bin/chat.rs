extern crate gpui_kit as gpui;

#[path = "chat/session.rs"]
mod session;
#[path = "chat/tray.rs"]
mod tray;

use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Icon, IconName, Sizable, Theme, ThemeMode,
        button::{Button, ButtonVariants},
        input::{InputEvent, Textarea, TextareaState},
        spinner::Spinner,
        text::TextView,
        tooltip::Tooltip,
    },
    prelude::*,
    *,
};
use session::{Conversation, Entry, Event};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

const EMPTY_CHAT_QUOTES: [&str; 10] = [
    "What shall we make clearer today?",
    "Bring me a thought, I’ll bring a plan",
    "What deserves your attention next?",
    "Ready when your next idea is",
    "What can I help untangle?",
    "Start anywhere, we’ll shape it together",
    "Where does today's thinking begin?",
    "Drop a rough idea, we'll sharpen it",
    "What's been sitting in the back of your mind?",
    "What are you curious about right now?",
];

fn empty_chat_quote(date: chrono::NaiveDate) -> &'static str {
    let hash = Sha256::digest(date.to_string().as_bytes());
    let seed = u64::from_be_bytes(hash[..8].try_into().expect("SHA-256 has at least 8 bytes"));
    EMPTY_CHAT_QUOTES[(seed % EMPTY_CHAT_QUOTES.len() as u64) as usize]
}

#[cfg(test)]
mod tests {
    use super::empty_chat_quote;

    #[test]
    fn welcome_quote_is_stable_for_each_calendar_date() {
        for (date, expected) in [
            ("2026-10-14", "What can I help untangle?"),
            ("2026-10-15", "What deserves your attention next?"),
            ("2026-12-31", "Ready when your next idea is"),
            ("2027-01-01", "Drop a rough idea, we'll sharpen it"),
        ] {
            let date = date.parse::<chrono::NaiveDate>().unwrap();
            assert_eq!(empty_chat_quote(date), expected);
        }
    }
}

gpui_kit::actions!(chat, [Quit, HideChat, OpenChat]);

struct Chat {
    conversation: Conversation,
    input: Entity<TextareaState>,
    scroll: ScrollHandle,
    requests: mpsc::Sender<String>,
    _input_subscription: Subscription,
    _events: Task<()>,
    _tray_events: Task<()>,
    _tray: Option<tray_icon::TrayIcon>,
    tray_error: Option<String>,
}

impl Chat {
    fn new(
        requests: mpsc::Sender<String>,
        mut events: mpsc::Receiver<Event>,
        mut commands: mpsc::UnboundedReceiver<tray::Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Message Alpha…")
                .auto_grow(2, 6)
                .submit_on_enter(true)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let subscription =
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter {
                    shift: false,
                    secondary: false,
                } => this.send(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            });
        let event_task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| {
                        let follow = this.scroll.max_offset().y + this.scroll.offset().y <= px(40.);
                        this.conversation.apply(event);
                        if follow {
                            this.scroll.scroll_to_bottom();
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.conversation.ready || this.conversation.error.is_none() {
                    this.conversation.apply(Event::Unavailable(
                        "Agent worker stopped. Quit and reopen to reconnect.".into(),
                    ));
                }
                cx.notify();
            });
        });
        let tray_task = cx.spawn_in(window, async move |this, cx| {
            while let Some(command) = commands.recv().await {
                if this
                    .update_in(cx, |this, window, cx| match command {
                        tray::Command::Open => this.show(window, cx),
                        tray::Command::Quit => cx.quit(),
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            conversation: Conversation::default(),
            input,
            scroll: ScrollHandle::new(),
            requests,
            _input_subscription: subscription,
            _events: event_task,
            _tray_events: tray_task,
            _tray: None,
            tray_error: None,
        }
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.activate(true);
        window.activate_window();
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation.busy || !self.conversation.ready {
            return;
        }
        let message = self.input.read(cx).value().trim().to_string();
        if message.is_empty() {
            return;
        }
        if self.requests.try_send(message.clone()).is_err() {
            self.conversation.apply(Event::Unavailable(
                "Agent worker is unavailable. Quit and reopen to reconnect.".into(),
            ));
        } else {
            self.conversation.start(message);
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    fn render_entry(&self, index: usize, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let border = cx.theme().border;
        let card = div()
            .group("chat-entry")
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded_lg()
            .min_w_0()
            .w_full()
            .flex_shrink_0();
        match entry {
            Entry::User(text) | Entry::Assistant { text, .. } => {
                let user = matches!(entry, Entry::User(_));
                let copy = text.clone();
                card.relative()
                    .pr_10()
                    .when(user, |card| card.border_1().border_color(border))
                    .child(
                        div().absolute().top_2().right_2().child(
                            div()
                                .id(("copy-hover", index))
                                .opacity(0.)
                                .group_hover("chat-entry", |style| style.opacity(1.))
                                .in_focus(|style| style.opacity(1.))
                                .child(
                                    Button::new(("copy", index))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Copy)
                                        .tooltip("Copy message")
                                        .accessibility_label("Copy message")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        }),
                                ),
                        ),
                    )
                    .child(
                        TextView::markdown(("message", index), text.clone())
                            .text_size(px(13.))
                            .selectable(true),
                    )
                    .into_any_element()
            }
            Entry::ToolIntent {
                intent, running, ..
            } => div()
                .flex()
                .items_center()
                .gap_2()
                .px_4()
                .flex_shrink_0()
                .text_size(px(13.))
                .text_color(cx.theme().muted_foreground)
                .child(div().flex_shrink_0().child(if *running {
                    Spinner::new()
                        .small()
                        .icon(Icon::default().data(include_bytes!("chat/loader.svg")))
                        .into_any_element()
                } else {
                    Icon::new(IconName::Search).small().into_any_element()
                }))
                .child(div().min_w_0().whitespace_normal().child(intent.clone()))
                .into_any_element(),
        }
    }
}

impl Render for Chat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let available = self.conversation.ready && self.conversation.error.is_none();
        let status = if self.conversation.busy {
            "Working…"
        } else if available {
            "Ready"
        } else if self.conversation.error.is_some() {
            "Unavailable"
        } else {
            "Connecting…"
        };
        let disabled = !self.conversation.ready
            || self.conversation.busy
            || self.input.read(cx).value().trim().is_empty();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .id("transcript")
                    .text_size(px(13.))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px_3()
                    .py_4()
                    .gap_3()
                    .when(self.conversation.entries.is_empty(), |view| {
                        view.child(
                            div()
                                .flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .p_5()
                                .text_center()
                                .whitespace_normal()
                                .text_color(cx.theme().muted_foreground)
                                .child(empty_chat_quote(chrono::Local::now().date_naive())),
                        )
                    })
                    .children(
                        self.conversation
                            .entries
                            .iter()
                            .enumerate()
                            .map(|(index, entry)| self.render_entry(index, entry, cx)),
                    )
                    .when(self.conversation.show_thinking(), |view| {
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_4()
                                .text_size(px(13.))
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    Spinner::new().small().icon(
                                        Icon::default().data(include_bytes!("chat/loader.svg")),
                                    ),
                                )
                                .child(div().min_w_0().whitespace_normal().child("Thinking...")),
                        )
                    }),
            )
            .when_some(self.tray_error.clone(), |view, error| {
                view.child(
                    div()
                        .px_5()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .when_some(self.conversation.error.clone(), |view, error| {
                view.child(
                    div()
                        .px_5()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Textarea::new(&self.input)
                            .bordered(false)
                            .appearance(false)
                            .aria_label("Message Alpha"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .id("connection-status")
                                    .tooltip(move |_, cx| cx.new(|_| Tooltip::new(status)).into())
                                    .child(
                                        if available {
                                            Icon::new(IconName::Globe).text_color(cx.theme().green)
                                        } else {
                                            Icon::default()
                                                .data(include_bytes!("chat/globe-off.svg"))
                                                .text_color(cx.theme().danger)
                                        }
                                        .small(),
                                    ),
                            )
                            .child(
                                Button::new("send")
                                    .primary()
                                    .icon(
                                        Icon::default()
                                            .data(include_bytes!("chat/send-horizontal.svg")),
                                    )
                                    .accessibility_label("Send message")
                                    .tooltip("Send message")
                                    .loading_icon(
                                        Icon::default().data(include_bytes!("chat/loader.svg")),
                                    )
                                    .loading(self.conversation.busy)
                                    .disabled(disabled)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.send(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (requests, request_rx) = mpsc::channel(1);
    let (events, event_rx) = mpsc::channel(128);
    let worker = runtime.spawn(session::run(request_rx, events));
    let (commands, command_rx) = mpsc::unbounded_channel();
    let startup_error = Rc::new(RefCell::new(None));
    let error = startup_error.clone();
    let app = gpui_kit::application().with_assets(assets::Assets);
    app.on_reopen(|cx| {
        cx.activate(true);
        if let Some(window) = cx.windows().first().copied() {
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_window_appearance(Some(WindowAppearance::Dark));
        #[cfg(target_os = "macos")]
        if let Err(err) = tray::set_app_icon() {
            *error.borrow_mut() = Some(err);
            cx.quit();
            return;
        }
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-w", HideChat, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Alpha".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Open Chat", OpenChat),
                MenuItem::action("Hide Chat", HideChat),
                MenuItem::separator(),
                MenuItem::action("Quit Alpha", Quit),
            ],
        }]);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(620.), px(760.)),
                cx,
            ))),
            window_min_size: Some(size(px(420.), px(400.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Alpha".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            let chat = cx.new(|cx| Chat::new(requests, event_rx, command_rx, window, cx));
            let weak = chat.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                // Keep the window and session alive; only explicit Quit ends the app.
                if weak
                    .upgrade()
                    .is_some_and(|chat| chat.read(cx)._tray.is_some())
                {
                    cx.hide();
                }
                false
            });
            let weak = chat.downgrade();
            window.on_next_frame(move |_, cx| {
                let result = tray::create(commands);
                let _ = weak.update(cx, |chat, cx| {
                    match result {
                        Ok(tray) => chat._tray = Some(tray),
                        Err(error) => {
                            chat.tray_error = Some(format!(
                                "Tray unavailable: {error:#}. Keep this window open; Cmd+Q quits."
                            ))
                        }
                    }
                    cx.notify();
                });
            });
            chat
        });
        match opened {
            Ok((window, chat)) => {
                let weak = chat.downgrade();
                cx.on_action(move |_: &OpenChat, cx| {
                    let _ = window.update(cx, |_, window, cx| {
                        let _ = weak.update(cx, |chat, cx| chat.show(window, cx));
                    });
                });
                let weak = chat.downgrade();
                cx.on_action(move |_: &HideChat, cx| {
                    if weak
                        .upgrade()
                        .is_some_and(|chat| chat.read(cx)._tray.is_some())
                    {
                        cx.hide();
                    }
                });
                cx.activate(true);
            }
            Err(err) => {
                *error.borrow_mut() = Some(err);
                cx.quit();
            }
        }
    });
    worker.abort();
    runtime.shutdown_timeout(Duration::from_secs(2));
    if let Some(error) = startup_error.borrow_mut().take() {
        return Err(error);
    }
    Ok(())
}
