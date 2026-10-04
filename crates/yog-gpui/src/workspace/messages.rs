use std::{collections::VecDeque, time::Duration};

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, RopeExt as _, Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants},
    input::{Textarea, TextareaState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div, point, prelude::FluentBuilder as _, px,
    relative,
};

const MAX_LINES: usize = 5_000;
const MAX_BYTES: usize = 1_048_576;
const MAX_LINE_BYTES: usize = 8_192;
const REFRESH_INTERVAL: Duration = Duration::from_millis(200);

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
    let mut start = 0;
    for (end, _) in pending.match_indices(['\r', '\n']) {
        if end > start {
            complete.push(pending[start..end].to_owned());
        }
        start = end + 1;
    }
    pending.drain(..start);
    complete
}

fn at_bottom(text: &TextareaState) -> bool {
    text.selected_range().is_empty()
        && text
            .visible_row_range()
            .is_none_or(|rows| rows.end >= text.text().lines_len())
}

pub struct Messages {
    entries: VecDeque<NewMessage>,
    retained_bytes: usize,
    discarded_lines: usize,
    removed_bytes: usize,
    removed_lines: usize,
    text_dirty: bool,
    refresh_pending: bool,
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
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .soft_wrap(false)
                .placeholder("No messages for this filter")
        });
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
            entries: VecDeque::new(),
            retained_bytes: 0,
            discarded_lines: 0,
            removed_bytes: 0,
            removed_lines: 0,
            text_dirty: false,
            refresh_pending: false,
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
        if visible && self.expanded {
            self.refresh_text(window, cx);
            if self.follow_on_show {
                let scroll = self.text.read(cx).scroll_offset();
                self.schedule_follow(scroll, window, cx);
            }
        }
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.entries.clear();
        self.retained_bytes = 0;
        self.discarded_lines = 0;
        self.removed_bytes = 0;
        self.removed_lines = 0;
        self.text_dirty = false;
        self.ffmpeg_partial.clear();
        self.follow_on_show = true;
        self.text
            .update(cx, |text, cx| text.set_value("", window, cx));
        cx.notify();
    }

    pub fn push(&mut self, message: NewMessage, window: &mut Window, cx: &mut Context<Self>) {
        if message.source == MessageSource::Ffmpeg {
            for line in complete_ffmpeg_lines(&mut self.ffmpeg_partial, &message.text) {
                self.push_line(MessageSource::Ffmpeg, message.level, line);
            }
            if self.ffmpeg_partial.len() > MAX_LINE_BYTES {
                let line = std::mem::take(&mut self.ffmpeg_partial);
                self.push_line(MessageSource::Ffmpeg, message.level, line);
            }
        } else {
            for line in message.text.lines() {
                self.push_line(message.source, message.level, line.to_owned());
            }
        }
        self.schedule_refresh(window, cx);
    }

    pub fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ffmpeg_partial.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.ffmpeg_partial);
        self.push_line(MessageSource::Ffmpeg, MessageLevel::Debug, line);
        self.schedule_refresh(window, cx);
    }

    fn push_line(&mut self, source: MessageSource, level: MessageLevel, mut text: String) {
        if text.trim().is_empty() {
            return;
        }
        if text.len() > MAX_LINE_BYTES {
            const SUFFIX: &str = "… [line truncated]";
            text.truncate(text.floor_char_boundary(MAX_LINE_BYTES - SUFFIX.len()));
            text.push_str(SUFFIX);
            text.shrink_to_fit();
        }
        let message = NewMessage {
            source,
            level,
            text,
        };
        self.text_dirty |= self.filter.matches(&message);
        self.retained_bytes += message.text.len();
        self.entries.push_back(message);
        while self.entries.len() > MAX_LINES || self.retained_bytes > MAX_BYTES {
            let removed = self.entries.pop_front().unwrap();
            self.retained_bytes -= removed.text.len();
            self.discarded_lines += 1;
            if self.filter.matches(&removed) {
                self.removed_bytes += self.filter.display_line(&removed).len();
                self.removed_lines += 1;
                self.text_dirty = true;
            }
        }
    }

    fn schedule_refresh(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.refresh_pending {
            return;
        }
        self.refresh_pending = true;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(REFRESH_INTERVAL).await;
            _ = this.update_in(cx, |this, window, cx| {
                this.refresh_pending = false;
                if this.expanded && this.page_visible {
                    this.refresh_text(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.text_dirty {
            return;
        }
        let text = self.text.read(cx);
        let selection = text.selected_range();
        let scroll = text.scroll_offset();
        let follow = at_bottom(text);
        let removed_bytes = self.removed_bytes;
        let line_height = cx.theme().font_size * 0.875 * 1.5;
        let scroll = point(
            scroll.x,
            (scroll.y + line_height * self.removed_lines as f32).min(px(0.)),
        );
        self.rebuild_text(window, cx);
        self.text.update(cx, |text, cx| {
            let len = text.text().len();
            let selection = if follow {
                len..len
            } else {
                selection.start.saturating_sub(removed_bytes).min(len)
                    ..selection.end.saturating_sub(removed_bytes).min(len)
            };
            text.set_selected_range(selection, cx);
            text.set_scroll_offset(scroll, cx);
        });
        if follow {
            self.schedule_follow(scroll, window, cx);
        }
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
            // Programmatic log output must not accumulate the editor's undo history.
            text.set_value(content, window, cx);
        });
        self.removed_bytes = 0;
        self.removed_lines = 0;
        self.text_dirty = false;
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
                                    .child(if self.discarded_lines == 0 {
                                        format!("{} lines", self.entries.len())
                                    } else {
                                        format!(
                                            "{} recent lines · {} older lines discarded",
                                            self.entries.len(),
                                            self.discarded_lines
                                        )
                                    }),
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
                                            this.refresh_text(window, cx);
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
                            .text_size(cx.theme().font_size * 0.875)
                            .line_height(relative(1.5))
                            .aria_label("Messages"),
                    ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        TestAppContext,
        component::Root,
        point, px, size,
        test::{TestAppContextExt as _, TestWindowExt as _},
    };

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
    fn closed_panel_defers_text_updates_until_opened(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut messages = None;
        let handle = cx.open_window(size(px(420.), px(240.)), |window, cx| {
            let view = cx.new(|cx| Messages::new(window, cx));
            messages = Some(view.clone());
            Root::new(view, window, cx)
        });
        let messages = messages.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Ffmpeg,
                        level: MessageLevel::Debug,
                        text: "first\nsecond\n".into(),
                    },
                    window,
                    cx,
                );
            });
            assert_eq!(messages.read(cx).text.read(cx).text().len(), 0);
            window.render_frame(cx);
            window.click("toggle-messages", cx);
            assert_eq!(
                messages.read(cx).text.read(cx).value().as_ref(),
                "[DEBUG] [FFmpeg] first\n[DEBUG] [FFmpeg] second\n"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn log_retention_evicts_old_lines_instead_of_growing_with_the_run(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.open_window(size(px(420.), px(240.)), |window, cx| {
            Root::new(cx.new(|cx| Messages::new(window, cx)), window, cx)
        });

        cx.update_window(handle.into(), |root, window, cx| {
            let messages = root
                .downcast::<Root>()
                .unwrap()
                .read(cx)
                .view()
                .clone()
                .downcast::<Messages>()
                .unwrap();
            messages.update(cx, |messages, cx| {
                messages.push(
                    NewMessage {
                        source: MessageSource::Ffmpeg,
                        level: MessageLevel::Debug,
                        text: (0..10_000).map(|index| format!("line {index}\n")).collect(),
                    },
                    window,
                    cx,
                );
                assert!(messages.entries.len() <= 5_000);
                assert_eq!(messages.entries.back().unwrap().text, "line 9999");
                assert!(!messages.entries.iter().any(|entry| entry.text == "line 0"));
                messages.filter = Filter::Ffmpeg;
                messages.rebuild_text(window, cx);
                assert!(!messages.text.read(cx).value().starts_with("line 0\n"));
                assert!(messages.text.read(cx).value().ends_with("line 9999\n"));
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn log_retention_bounds_bytes_and_preserves_utf8(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.open_window(size(px(420.), px(240.)), |window, cx| {
            Root::new(cx.new(|cx| Messages::new(window, cx)), window, cx)
        });

        cx.update_window(handle.into(), |root, window, cx| {
            let messages = root
                .downcast::<Root>()
                .unwrap()
                .read(cx)
                .view()
                .clone()
                .downcast::<Messages>()
                .unwrap();
            messages.update(cx, |messages, cx| {
                let line = format!("{}\n", "日志".repeat(3_000));
                messages.push(
                    NewMessage {
                        source: MessageSource::Ffmpeg,
                        level: MessageLevel::Debug,
                        text: line.repeat(200),
                    },
                    window,
                    cx,
                );
                let retained: usize = messages.entries.iter().map(|entry| entry.text.len()).sum();
                assert!(retained <= 1_048_576, "retained {retained} bytes");
                assert!(
                    messages
                        .entries
                        .iter()
                        .all(|entry| entry.text.len() <= 8_192)
                );
                assert!(
                    messages
                        .entries
                        .back()
                        .unwrap()
                        .text
                        .ends_with("[line truncated]")
                );
            });
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn continuous_log_stream_keeps_the_panel_bounded(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut messages = None;
        let handle = cx.open_window(size(px(420.), px(240.)), |window, cx| {
            let view = cx.new(|cx| Messages::new(window, cx));
            messages = Some(view.clone());
            Root::new(view, window, cx)
        });
        let messages = messages.unwrap();
        let chunk = format!("{}\n", "x".repeat(63)).repeat(128);
        let started = std::time::Instant::now();
        cx.update_window(handle.into(), |_, window, cx| {
            messages.update(cx, |messages, cx| {
                for _ in 0..16_384 {
                    messages.push(
                        NewMessage {
                            source: MessageSource::Ffmpeg,
                            level: MessageLevel::Debug,
                            text: chunk.clone(),
                        },
                        window,
                        cx,
                    );
                }
                assert_eq!(messages.entries.len(), 5_000);
                assert_eq!(messages.discarded_lines, 2_097_152 - 5_000);
                assert!(messages.retained_bytes <= MAX_BYTES);
                assert_eq!(messages.text.read(cx).text().len(), 0);
                messages.expanded = true;
                messages.refresh_text(window, cx);
                assert!(messages.text.read(cx).text().len() <= MAX_BYTES);
                messages.clear(window, cx);
                assert_eq!(messages.retained_bytes, 0);
                assert_eq!(messages.discarded_lines, 0);
                assert_eq!(messages.text.read(cx).text().len(), 0);
            });
        })
        .unwrap();
        eprintln!(
            "128 MiB simulated log stream processed in {:?}",
            started.elapsed()
        );
    }

    #[gpui_kit::test]
    async fn toolbar_stays_above_selectable_text_and_copies_its_contents(cx: &mut TestAppContext) {
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
        })
        .unwrap();
        cx.wait_for(window_handle.into(), Duration::from_secs(1), |_, cx| {
            messages.read(cx).text.read(cx).text().len() > 0
        })
        .await;
        let mut selected = None;
        cx.update_window(window_handle.into(), |_, window, cx| {
            window.render_frame(cx);

            let toolbar_bottom = window.find("copy-messages").bounds().bottom();
            let text_id = ("input", messages.read(cx).text.entity_id());
            let text_top = window.find(text_id).bounds().top();
            assert!(toolbar_bottom <= text_top);

            window.click(text_id, cx);
            window.press("secondary-a", cx);
            let text = messages.read(cx).text.clone();
            let state = text.read(cx);
            let selection = state.selected_range();
            assert_eq!(selection, 0..state.text().len());
            selected = Some(selection);
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
        })
        .unwrap();
        cx.wait_for(window_handle.into(), Duration::from_secs(1), |_, cx| {
            messages
                .read(cx)
                .text
                .read(cx)
                .value()
                .contains("Continuing")
        })
        .await;
        cx.update_window(window_handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                messages.read(cx).text.read(cx).selected_range(),
                selected.unwrap()
            );

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
    async fn manual_scroll_is_not_overridden_by_new_messages(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut messages = None;
        let window_handle = cx.open_window(size(px(420.), px(240.)), |window, cx| {
            let view = cx.new(|cx| Messages::new(window, cx));
            messages = Some(view.clone());
            Root::new(view, window, cx)
        });
        let messages = messages.unwrap();
        let mut bottom = px(0.);

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
        })
        .unwrap();
        cx.wait_for(window_handle.into(), Duration::from_secs(1), |_, cx| {
            messages.read(cx).text.read(cx).text().len() > 0
        })
        .await;
        cx.update_window(window_handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);
            window.simulate_next_frame(cx);
            window.render_frame(cx);

            let text = messages.read(cx).text.clone();
            let toolbar_bottom = window.find("copy-messages").bounds().bottom();
            let text_top = window.find(("input", text.entity_id())).bounds().top();
            assert!(toolbar_bottom <= text_top);
            bottom = text.read(cx).scroll_offset().y;
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
        })
        .unwrap();
        cx.wait_for(window_handle.into(), Duration::from_secs(1), |_, cx| {
            messages
                .read(cx)
                .text
                .read(cx)
                .value()
                .contains("followed line")
        })
        .await;
        cx.update_window(window_handle.into(), |_, window, cx| {
            let text = messages.read(cx).text.clone();
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
        })
        .unwrap();
        cx.wait_for(window_handle.into(), Duration::from_secs(1), |_, cx| {
            messages.read(cx).text.read(cx).value().contains("new line")
        })
        .await;
        cx.update_window(window_handle.into(), |_, window, cx| {
            let text = messages.read(cx).text.clone();
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
