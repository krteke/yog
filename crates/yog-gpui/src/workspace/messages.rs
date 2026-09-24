use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, RopeExt as _, Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants},
    input::{Textarea, TextareaState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, relative,
};

const FILTERS: &[&str] = &[
    "All messages",
    "Program",
    "FFmpeg",
    "Commands",
    "Warnings",
    "Errors",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageSource {
    Program,
    Ffmpeg,
    Command,
}

impl MessageSource {
    fn label(self) -> &'static str {
        match self {
            Self::Program => "Program",
            Self::Ffmpeg => "FFmpeg",
            Self::Command => "Command",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl MessageLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warning => "WARN",
            Self::Error => "ERROR",
        }
    }
}

pub struct NewMessage {
    pub source: MessageSource,
    pub level: MessageLevel,
    pub text: String,
}

pub struct ExpansionChanged;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Program,
    Ffmpeg,
    Commands,
    Warnings,
    Errors,
}

impl Filter {
    fn from_label(label: &str) -> Self {
        match label {
            "All messages" => Self::All,
            "Program" => Self::Program,
            "FFmpeg" => Self::Ffmpeg,
            "Commands" => Self::Commands,
            "Warnings" => Self::Warnings,
            "Errors" => Self::Errors,
            _ => unreachable!("message filters are internal"),
        }
    }

    fn matches(self, message: &NewMessage) -> bool {
        match self {
            Self::All => true,
            Self::Program => message.source == MessageSource::Program,
            Self::Ffmpeg => message.source == MessageSource::Ffmpeg,
            Self::Commands => message.source == MessageSource::Command,
            Self::Warnings => message.level == MessageLevel::Warning,
            Self::Errors => message.level == MessageLevel::Error,
        }
    }

    fn display_line(self, message: &NewMessage) -> String {
        match self {
            Self::All => format!(
                "[{}] [{}] {}\n",
                message.level.label(),
                message.source.label(),
                message.text
            ),
            Self::Program => format!("[{}] {}\n", message.level.label(), message.text),
            Self::Ffmpeg | Self::Commands => format!("{}\n", message.text),
            Self::Warnings | Self::Errors => {
                format!("[{}] {}\n", message.source.label(), message.text)
            }
        }
    }
}

fn complete_ffmpeg_lines(pending: &mut String, chunk: &str) -> Vec<String> {
    pending.push_str(chunk);
    let mut complete = Vec::new();
    while let Some(end) = pending.find(['\r', '\n']) {
        let line = pending[..end].to_owned();
        pending.drain(..=end);
        if !line.is_empty() {
            complete.push(line);
        }
    }
    complete
}

fn at_bottom(text: &TextareaState) -> bool {
    text.selected_range().is_empty()
        && text
            .visible_row_range()
            .is_none_or(|rows| rows.end >= text.text().lines_len())
}

pub struct Messages {
    entries: Vec<NewMessage>,
    ffmpeg_partial: String,
    expanded: bool,
    filter: Filter,
    filter_select: Entity<SelectState<SearchableVec<&'static str>>>,
    text: Entity<TextareaState>,
    page_visible: bool,
    follow_on_show: bool,
    follow_pending: bool,
    _filter_subscription: Subscription,
}

impl EventEmitter<ExpansionChanged> for Messages {}

impl Messages {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(FILTERS.to_vec()),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let text =
            cx.new(|cx| TextareaState::new(window, cx).placeholder("No messages for this filter"));
        let filter_subscription = cx.subscribe_in(
            &filter_select,
            window,
            |this, _, _: &SelectEvent<SearchableVec<&'static str>>, window, cx| {
                let label = this
                    .filter_select
                    .read(cx)
                    .selected_value()
                    .expect("message filter always has a selection");
                this.filter = Filter::from_label(label);
                this.rebuild_text(window, cx);
                cx.notify();
            },
        );
        Self {
            entries: Vec::new(),
            ffmpeg_partial: String::new(),
            expanded: false,
            filter: Filter::All,
            filter_select,
            text,
            page_visible: true,
            follow_on_show: true,
            follow_pending: false,
            _filter_subscription: filter_subscription,
        }
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn set_page_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.page_visible == visible {
            return;
        }
        if !visible && self.expanded {
            self.follow_on_show = at_bottom(self.text.read(cx));
        }
        self.page_visible = visible;
        if visible && self.expanded && self.follow_on_show {
            let scroll = self.text.read(cx).scroll_offset();
            self.schedule_follow(scroll, window, cx);
        }
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.entries.clear();
        self.ffmpeg_partial.clear();
        self.follow_on_show = true;
        self.text
            .update(cx, |text, cx| text.set_value("", window, cx));
        cx.notify();
    }

    pub fn push(&mut self, message: NewMessage, window: &mut Window, cx: &mut Context<Self>) {
        let mut appended = String::new();
        if message.source == MessageSource::Ffmpeg {
            for line in complete_ffmpeg_lines(&mut self.ffmpeg_partial, &message.text) {
                self.push_line(MessageSource::Ffmpeg, message.level, line, &mut appended);
            }
            if self.ffmpeg_partial.len() > 8_192 {
                let line = std::mem::take(&mut self.ffmpeg_partial);
                self.push_line(MessageSource::Ffmpeg, message.level, line, &mut appended);
            }
        } else {
            for line in message.text.lines() {
                self.push_line(
                    message.source,
                    message.level,
                    line.to_owned(),
                    &mut appended,
                );
            }
        }
        self.append_text(appended, window, cx);
    }

    pub fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ffmpeg_partial.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.ffmpeg_partial);
        let mut appended = String::new();
        self.push_line(
            MessageSource::Ffmpeg,
            MessageLevel::Debug,
            line,
            &mut appended,
        );
        self.append_text(appended, window, cx);
    }

    fn push_line(
        &mut self,
        source: MessageSource,
        level: MessageLevel,
        text: String,
        appended: &mut String,
    ) {
        if text.trim().is_empty() {
            return;
        }
        let message = NewMessage {
            source,
            level,
            text,
        };
        if self.filter.matches(&message) {
            appended.push_str(&self.filter.display_line(&message));
        }
        self.entries.push(message);
    }

    fn append_text(&mut self, appended: String, window: &mut Window, cx: &mut Context<Self>) {
        if appended.is_empty() {
            return;
        }
        let rendered = self.expanded && self.page_visible;
        let follow_on_show = self.follow_on_show;
        let mut follow_from = None;
        self.text.update(cx, |text, cx| {
            let selection = text.selected_range();
            let scroll = text.scroll_offset();
            let was_at_bottom = if rendered {
                at_bottom(text)
            } else {
                follow_on_show
            };
            let end = text.text().len();
            text.set_selected_range(end..end, cx);
            text.insert(appended, window, cx);
            if was_at_bottom && rendered {
                follow_from = Some(scroll);
            } else if !was_at_bottom {
                text.set_selected_range(selection, cx);
                text.set_scroll_offset(scroll, cx);
            }
        });
        if let Some(scroll) = follow_from {
            self.schedule_follow(scroll, window, cx);
        }
        cx.notify();
    }

    fn schedule_follow(
        &mut self,
        scroll: gpui_kit::Point<gpui_kit::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.follow_pending {
            return;
        }
        self.follow_pending = true;
        let view = cx.entity();
        window.on_next_frame(move |_, cx| {
            view.update(cx, |this, cx| {
                this.follow_pending = false;
                if !this.expanded || !this.page_visible {
                    return;
                }
                this.text.update(cx, |text, cx| {
                    if text.selected_range().is_empty() && text.scroll_offset().y <= scroll.y {
                        let end = text.text().len();
                        text.set_selected_range(end..end, cx);
                    }
                });
            });
        });
    }

    fn rebuild_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut content = String::new();
        for message in &self.entries {
            if self.filter.matches(message) {
                content.push_str(&self.filter.display_line(message));
            }
        }
        self.text.update(cx, |text, cx| {
            text.set_value(content, window, cx);
        });
    }
}

impl Render for Messages {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .when(self.expanded, |this| this.h_full())
            .bg(cx.theme().background)
            .child(
                div()
                    .flex_none()
                    .min_w_0()
                    .px_4()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(!self.expanded, |this| this.border_t_1())
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().font_medium().child("Messages"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{} total lines", self.entries.len())),
                            )
                            .child(div().flex_1().min_w_0())
                            .child(
                                Button::new("toggle-messages")
                                    .ghost()
                                    .small()
                                    .label(if self.expanded { "Collapse" } else { "Expand" })
                                    .icon(Icon::new(if self.expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronUp
                                    }))
                                    .accessibility_label(if self.expanded {
                                        "Collapse messages"
                                    } else {
                                        "Expand messages"
                                    })
                                    .tooltip(if self.expanded {
                                        "Collapse messages"
                                    } else {
                                        "Expand messages"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.expanded {
                                            this.follow_on_show = at_bottom(this.text.read(cx));
                                            this.expanded = false;
                                        } else {
                                            this.expanded = true;
                                            if this.follow_on_show {
                                                let scroll = this.text.read(cx).scroll_offset();
                                                this.schedule_follow(scroll, window, cx);
                                            }
                                        }
                                        cx.emit(ExpansionChanged);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.expanded, |this| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_end()
                                .gap_2()
                                .child(
                                    div().w_32().flex_none().child(
                                        Select::new(&self.filter_select)
                                            .id("messages-filter")
                                            .accessibility_label("Message filter")
                                            .w_full(),
                                    ),
                                )
                                .child(
                                    Button::new("copy-messages")
                                        .ghost()
                                        .small()
                                        .label("Copy all")
                                        .disabled(self.text.read(cx).text().len() == 0)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let content = this.text.read(cx).value().to_string();
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                content,
                                            ));
                                        })),
                                ),
                        )
                    }),
            )
            .when(self.expanded, |this| {
                this.child(
                    div().flex_1().min_h_0().min_w_0().child(
                        Textarea::new(&self.text)
                            .readonly(true)
                            .appearance(false)
                            .bordered(false)
                            .h(relative(1.))
                            .w_full()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_sm()
                            .aria_label("Messages"),
                    ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, component::Root, point, px, size, test::TestWindowExt as _};

    #[test]
    fn ffmpeg_chunks_preserve_partial_lines_and_normalize_crlf() {
        let mut pending = String::new();
        let first = complete_ffmpeg_lines(&mut pending, "frame=1\n[error] inva");
        assert_eq!(first, ["frame=1"]);
        assert_eq!(pending, "[error] inva");

        let second = complete_ffmpeg_lines(&mut pending, "lid input\r\nlast line");
        assert_eq!(second, ["[error] invalid input"]);
        assert_eq!(pending, "last line");
    }

    #[test]
    fn ffmpeg_filter_keeps_raw_lines_while_all_messages_identifies_their_source() {
        let message = NewMessage {
            source: MessageSource::Ffmpeg,
            level: MessageLevel::Debug,
            text: "frame=10".into(),
        };
        assert_eq!(Filter::Ffmpeg.display_line(&message), "frame=10\n");
        assert_eq!(
            Filter::All.display_line(&message),
            "[DEBUG] [FFmpeg] frame=10\n"
        );
    }

    #[gpui_kit::test]
    fn toolbar_stays_above_selectable_text_and_copies_its_contents(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut messages = None;
        let window_handle = cx.open_window(size(px(420.), px(400.)), |window, cx| {
            let view = cx.new(|cx| Messages::new(window, cx));
            messages = Some(view.clone());
            Root::new(view, window, cx)
        });
        let messages = messages.unwrap();

        cx.update_window(window_handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let text_id = ("input", messages.read(cx).text.entity_id());
            assert!(window.try_find(text_id).is_none());
            window.click("toggle-messages", cx);
            assert!(messages.read(cx).is_expanded());

            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Warning,
                        text: "Cannot open input".into(),
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);

            let toolbar_bottom = window.find("copy-messages").bounds().bottom();
            let text_top = window.find(text_id).bounds().top();
            assert!(toolbar_bottom <= text_top);

            window.click(text_id, cx);
            window.press("secondary-a", cx);
            let text = messages.read(cx).text.clone();
            let state = text.read(cx);
            let selected = state.selected_range();
            assert_eq!(selected, 0..state.text().len());
            window.press("secondary-c", cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("[WARN] [Program] Cannot open input\n".into())
            );

            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Info,
                        text: "Continuing".into(),
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            assert_eq!(text.read(cx).selected_range(), selected);

            window.click("copy-messages", cx);
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("[WARN] [Program] Cannot open input\n[INFO] [Program] Continuing\n".into())
            );
        })
        .unwrap();
        cx.update_window(window_handle.into(), |_, window, cx| {
            window.click("toggle-messages", cx);
            assert!(!messages.read(cx).is_expanded());
            assert!(window.try_find("messages-filter").is_none());
            window.click("toggle-messages", cx);
            assert_eq!(
                messages.read(cx).text.read(cx).value().as_ref(),
                "[WARN] [Program] Cannot open input\n[INFO] [Program] Continuing\n"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn filter_menu_changes_displayed_messages(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut messages = None;
        let window_handle = cx.open_window(size(px(420.), px(400.)), |window, cx| {
            let view = cx.new(|cx| Messages::new(window, cx));
            messages = Some(view.clone());
            Root::new(view, window, cx)
        });
        let messages = messages.unwrap();

        cx.update_window(window_handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("toggle-messages", cx);
            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Info,
                        text: "Starting".into(),
                    },
                    window,
                    cx,
                );
                messages.push(
                    NewMessage {
                        source: MessageSource::Ffmpeg,
                        level: MessageLevel::Debug,
                        text: "frame=1\n".into(),
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.click("messages-filter", cx);
            assert_eq!(window.find("messages-filter").expanded(), Some(true));
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(
                messages.read(cx).text.read(cx).value().as_ref(),
                "[INFO] Starting\n"
            );
        });
    }

    #[gpui_kit::test]
    fn closed_panel_opens_at_latest_message(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut messages = None;
        let handle = cx.open_window(size(px(420.), px(240.)), |window, cx| {
            let view = cx.new(|cx| Messages::new(window, cx));
            messages = Some(view.clone());
            Root::new(view, window, cx)
        });
        let messages = messages.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let lines = (0..100)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n");
            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Info,
                        text: lines,
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.click("toggle-messages", cx);
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            let bottom = messages.read(cx).text.read(cx).scroll_offset().y;
            assert!(bottom < px(0.));
            assert_eq!(
                messages
                    .read(cx)
                    .text
                    .read(cx)
                    .visible_row_range()
                    .unwrap()
                    .end,
                messages.read(cx).text.read(cx).text().lines_len()
            );

            messages.update(cx, |messages, cx| {
                messages.set_page_visible(false, window, cx);
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Info,
                        text: (100..120)
                            .map(|index| format!("line {index}"))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    },
                    window,
                    cx,
                );
                messages.set_page_visible(true, window, cx);
            });
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            assert!(messages.read(cx).text.read(cx).scroll_offset().y < bottom);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn manual_scroll_is_not_overridden_by_new_messages(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut messages = None;
        let window_handle = cx.open_window(size(px(420.), px(240.)), |window, cx| {
            let view = cx.new(|cx| Messages::new(window, cx));
            messages = Some(view.clone());
            Root::new(view, window, cx)
        });
        let messages = messages.unwrap();

        cx.update_window(window_handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("toggle-messages", cx);
            let lines = (0..100)
                .map(|index| format!("line {index}"))
                .collect::<Vec<_>>()
                .join("\n");
            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Info,
                        text: lines,
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);

            let text = messages.read(cx).text.clone();
            let toolbar_bottom = window.find("copy-messages").bounds().bottom();
            let text_top = window.find(("input", text.entity_id())).bounds().top();
            assert!(toolbar_bottom <= text_top);
            let bottom = text.read(cx).scroll_offset().y;
            assert!(bottom < px(0.));
            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Info,
                        text: "followed line".into(),
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            assert!(text.read(cx).scroll_offset().y < bottom);

            text.update(cx, |text, cx| {
                text.set_scroll_offset(point(px(0.), px(0.)), cx);
            });
            window.render_frame(cx);
            assert_eq!(text.read(cx).scroll_offset().y, px(0.));

            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Info,
                        text: "new line".into(),
                    },
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            assert_eq!(text.read(cx).scroll_offset().y, px(0.));

            window.click("toggle-messages", cx);
            window.click("toggle-messages", cx);
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            assert_eq!(text.read(cx).scroll_offset().y, px(0.));

            messages.update(cx, |messages, cx| {
                messages.set_page_visible(false, window, cx);
                messages.push(
                    NewMessage {
                        source: MessageSource::Program,
                        level: MessageLevel::Info,
                        text: "while on another page".into(),
                    },
                    window,
                    cx,
                );
                messages.set_page_visible(true, window, cx);
            });
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            assert_eq!(text.read(cx).scroll_offset().y, px(0.));
        })
        .unwrap();
    }
}
