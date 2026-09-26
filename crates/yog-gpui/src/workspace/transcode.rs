mod settings;

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants},
    progress::Progress as ProgressBar,
    scroll::ScrollableElement,
    tag::Tag,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
#[cfg(test)]
use gpui_kit::{InteractiveElement as _, test::TestSupportExt as _};
use tokio_util::sync::CancellationToken;
use yog_core::ffmpeg::progress::Progress;
use yog_runtime::{
    RunOutcome, RunStatus,
    event::{RunEvent, RunPhase, TaskStatus, VmafOutcome},
};

use self::settings::TranscodeSettings;
use super::{
    components,
    execution::{self, Activity, ActivityChanged, WorkerMessage},
    messages::{ExpansionChanged, MessageLevel, MessageSource, Messages, NewMessage},
    settings::SettingsPanel,
    source::SourcePicker,
};

enum Stage {
    Idle,
    Running,
    Cancelling,
    Finished(RunStatus),
}

struct TaskResult {
    name: String,
    status: ResultStatus,
    error: Option<String>,
}

enum ResultStatus {
    Success,
    Failure,
    Cancelled,
    Skipped,
}

pub struct TranscodePage {
    source: Entity<SourcePicker>,
    settings: Entity<TranscodeSettings>,
    messages: Entity<Messages>,
    stage: Stage,
    cancellation: Option<CancellationToken>,
    error: Option<String>,
    notice: Option<(MessageLevel, String)>,
    phase: Option<RunPhase>,
    current_name: Option<String>,
    current_output: Option<PathBuf>,
    completed_output: Option<PathBuf>,
    task_index: usize,
    task_total: usize,
    succeeded: usize,
    failed: usize,
    skipped: usize,
    duration: Option<std::time::Duration>,
    progress: Progress,
    started: Option<Instant>,
    finished_elapsed: Option<Duration>,
    results: Vec<TaskResult>,
    _source_observation: Subscription,
    _settings_observation: Subscription,
    _messages_subscription: Subscription,
}

impl EventEmitter<ActivityChanged> for TranscodePage {}

impl TranscodePage {
    pub fn new(
        source: Entity<SourcePicker>,
        preferences: Entity<SettingsPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = cx.new(|cx| TranscodeSettings::new(source.clone(), preferences, window, cx));
        let messages = cx.new(|cx| Messages::new(window, cx));
        let source_observation = cx.observe(&source, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let messages_subscription =
            cx.subscribe(&messages, |_, _, _: &ExpansionChanged, cx| cx.notify());
        Self {
            source,
            settings,
            messages,
            stage: Stage::Idle,
            cancellation: None,
            error: None,
            notice: None,
            phase: None,
            current_name: None,
            current_output: None,
            completed_output: None,
            task_index: 0,
            task_total: 1,
            succeeded: 0,
            failed: 0,
            skipped: 0,
            duration: None,
            progress: Progress::default(),
            started: None,
            finished_elapsed: None,
            results: Vec::new(),
            _source_observation: source_observation,
            _settings_observation: settings_observation,
            _messages_subscription: messages_subscription,
        }
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Running | Stage::Cancelling) {
            return;
        }
        let request = match self.settings.read(cx).build(cx) {
            Ok(request) => request,
            Err(error) => {
                self.message(MessageLevel::Error, error.clone(), window, cx);
                self.error = Some(error);
                cx.notify();
                return;
            }
        };

        self.stage = Stage::Running;
        self.error = None;
        self.notice = None;
        self.phase = None;
        self.current_name = None;
        self.current_output = None;
        self.completed_output = None;
        self.task_index = 0;
        self.task_total = 1;
        self.succeeded = 0;
        self.failed = 0;
        self.skipped = 0;
        self.duration = None;
        self.progress = Progress::default();
        self.started = Some(Instant::now());
        self.finished_elapsed = None;
        self.results.clear();
        self.messages.update(cx, |messages, cx| {
            messages.clear(window, cx);
            messages.push(
                NewMessage {
                    source: MessageSource::Program,
                    level: MessageLevel::Info,
                    text: "Starting transcode".into(),
                },
                window,
                cx,
            );
        });

        let cancellation = CancellationToken::new();
        self.cancellation = Some(cancellation.clone());
        let mut receiver = execution::spawn(request, cancellation);
        cx.spawn(async move |this, cx| {
            while let Some(message) = receiver.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.handle_message(message, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.emit(ActivityChanged);
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
            self.stage = Stage::Cancelling;
            self.message(
                MessageLevel::Info,
                "Cancellation requested".into(),
                window,
                cx,
            );
            cx.emit(ActivityChanged);
            cx.notify();
        }
    }

    fn message(
        &self,
        level: MessageLevel,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.messages.update(cx, |messages, cx| {
            messages.push(
                NewMessage {
                    source: MessageSource::Program,
                    level,
                    text,
                },
                window,
                cx,
            );
        });
    }

    fn handle_message(
        &mut self,
        message: WorkerMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match message {
            WorkerMessage::Log(log) => {
                self.messages
                    .update(cx, |messages, cx| messages.push(log, window, cx));
            }
            WorkerMessage::Event(event) => match event {
                RunEvent::PhaseChanged(phase) => {
                    self.phase = Some(phase);
                    self.message(MessageLevel::Info, self.phase_label().into(), window, cx);
                }
                RunEvent::BatchDiscovered { total, skipped } => {
                    self.task_total = total;
                    self.skipped = skipped;
                    self.message(
                        MessageLevel::Info,
                        format!("Discovered {total} video files; {skipped} skipped"),
                        window,
                        cx,
                    );
                }
                RunEvent::InputSkipped { input, error } => {
                    self.message(
                        MessageLevel::Warning,
                        format!("Skipped {}: {error}", input.display()),
                        window,
                        cx,
                    );
                    self.results.push(TaskResult {
                        name: input.display().to_string(),
                        status: ResultStatus::Skipped,
                        error: Some(error),
                    });
                }
                RunEvent::TaskStarted {
                    index,
                    total,
                    input,
                    output,
                } => {
                    self.task_index = index;
                    self.task_total = total;
                    self.current_name = Some(input.display().to_string());
                    self.current_output = output;
                    self.notice = None;
                    self.duration = None;
                    self.progress = Progress::default();
                    self.message(
                        MessageLevel::Info,
                        format!("Task {index}/{total}: {}", input.display()),
                        window,
                        cx,
                    );
                }
                RunEvent::Progress { duration, progress } => {
                    self.duration = duration;
                    self.progress = progress;
                }
                RunEvent::VmafFinished {
                    output, outcome, ..
                } => match outcome {
                    VmafOutcome::Scored(score) => {
                        self.notice = Some((MessageLevel::Info, format!("VMAF {score:.2}")));
                        self.message(
                            MessageLevel::Info,
                            format!("VMAF {score:.2}: {}", output.display()),
                            window,
                            cx,
                        );
                    }
                    VmafOutcome::Failed(error) => {
                        self.notice =
                            Some((MessageLevel::Warning, format!("VMAF failed: {error}")));
                    }
                    VmafOutcome::Cancelled => {
                        self.notice = Some((MessageLevel::Warning, "VMAF cancelled".into()));
                    }
                },
                RunEvent::Warning { message } => {
                    self.message(MessageLevel::Warning, message.clone(), window, cx);
                    self.notice = Some((MessageLevel::Warning, message));
                }
                RunEvent::Error { message } => {
                    self.message(MessageLevel::Error, message.clone(), window, cx);
                    self.notice = Some((MessageLevel::Error, message));
                }
                RunEvent::TaskFinished {
                    input,
                    total,
                    status,
                    error,
                    ..
                } => {
                    let (level, label) = match status {
                        TaskStatus::Success => (MessageLevel::Info, "Completed"),
                        TaskStatus::Failure => (MessageLevel::Error, "Failed"),
                        TaskStatus::Cancelled => (MessageLevel::Warning, "Cancelled"),
                    };
                    self.message(
                        level,
                        format!(
                            "{label}: {}{}",
                            input.display(),
                            error
                                .as_ref()
                                .map_or(String::new(), |error| format!(": {error}"))
                        ),
                        window,
                        cx,
                    );
                    match status {
                        TaskStatus::Success => self.succeeded += 1,
                        TaskStatus::Failure => self.failed += 1,
                        TaskStatus::Cancelled => {}
                    }
                    if status == TaskStatus::Success && total == 1 {
                        self.completed_output = self.current_output.clone();
                    }
                    if total == 1 || status != TaskStatus::Success {
                        self.results.push(TaskResult {
                            name: input.display().to_string(),
                            status: match status {
                                TaskStatus::Success => ResultStatus::Success,
                                TaskStatus::Failure => ResultStatus::Failure,
                                TaskStatus::Cancelled => ResultStatus::Cancelled,
                            },
                            error,
                        });
                    }
                }
                _ => {}
            },
            WorkerMessage::Finished(outcome) => {
                self.messages
                    .update(cx, |messages, cx| messages.finish(window, cx));
                self.stage = Stage::Finished(outcome.status());
                self.finished_elapsed = self.started.map(|started| started.elapsed());
                self.cancellation = None;
                let failure = match outcome {
                    RunOutcome::Failed(error) => {
                        let detail = format!("{error:#}");
                        self.error = Some(detail.clone());
                        Some(detail)
                    }
                    RunOutcome::Batch(batch) => {
                        self.task_total = batch.total;
                        self.succeeded = batch.succeeded;
                        self.failed = batch.failures.len();
                        None
                    }
                    RunOutcome::Completed | RunOutcome::Cancelled => None,
                };
                let (level, label) = match self.stage {
                    Stage::Finished(RunStatus::Success) => (MessageLevel::Info, "Run completed"),
                    Stage::Finished(RunStatus::Failure) => (MessageLevel::Error, "Run failed"),
                    Stage::Finished(RunStatus::Cancelled) => {
                        (MessageLevel::Warning, "Run cancelled")
                    }
                    Stage::Idle | Stage::Running | Stage::Cancelling => unreachable!(),
                };
                self.message(
                    level,
                    failure.map_or_else(|| label.to_owned(), |error| format!("{label}: {error}")),
                    window,
                    cx,
                );
            }
        }
        cx.emit(ActivityChanged);
        cx.notify();
    }

    fn phase_label(&self) -> &'static str {
        if self.phase == Some(RunPhase::Planning) {
            "Preparing transcode"
        } else {
            execution::phase_label(self.phase)
        }
    }

    pub fn activity(&self) -> Option<Activity> {
        match self.stage {
            Stage::Idle => None,
            Stage::Running => {
                let mut label = self.phase_label().to_owned();
                if self.task_index > 0 {
                    label.push_str(&format!(" · {}/{}", self.task_index, self.task_total));
                }
                if self.phase == Some(RunPhase::Transcoding)
                    && let Some(percent) = self.progress_percent()
                {
                    label.push_str(&format!(" · {percent:.0}%"));
                }
                Some(Activity::Running(label))
            }
            Stage::Cancelling => Some(Activity::Cancelling),
            Stage::Finished(status) => Some(Activity::Finished(status)),
        }
    }

    pub fn set_page_visible(&self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.messages.update(cx, |messages, cx| {
            messages.set_page_visible(visible, window, cx);
        });
    }

    fn progress_percent(&self) -> Option<f32> {
        let duration = self.duration?;
        let elapsed = self.progress.out_time_us?;
        if duration.is_zero() {
            return None;
        }
        Some((elapsed.max(0) as f64 / duration.as_micros() as f64 * 100.).clamp(0.0, 100.0) as f32)
    }

    fn render_run_summary(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (headline, state, tag) = match self.stage {
            Stage::Idle => unreachable!("idle runs have no summary"),
            Stage::Running => (self.phase_label(), "Running", Tag::info()),
            Stage::Cancelling => ("Cancelling transcode", "Cancelling", Tag::warning()),
            Stage::Finished(RunStatus::Success) => {
                ("Transcode complete", "Completed", Tag::success())
            }
            Stage::Finished(RunStatus::Failure) => ("Transcode failed", "Failed", Tag::danger()),
            Stage::Finished(RunStatus::Cancelled) => {
                ("Transcode cancelled", "Cancelled", Tag::secondary())
            }
        };
        let active = matches!(self.stage, Stage::Running | Stage::Cancelling);
        let progress = (self.phase == Some(RunPhase::Transcoding))
            .then(|| self.progress_percent())
            .flatten();
        let current_file = (if active || self.task_total == 1 {
            self.current_name.as_ref()
        } else {
            None
        })
        .and_then(|name| {
            Path::new(name)
                .file_name()
                .map(|file| file.to_string_lossy().into_owned())
        });
        let mut metrics = Vec::new();
        if let Some(elapsed) = self
            .finished_elapsed
            .or_else(|| self.started.map(|time| time.elapsed()))
        {
            metrics.push(("Elapsed", components::format_duration(elapsed)));
        }
        if active && let Some(speed) = self.progress.speed {
            metrics.push(("Speed", format!("{speed:.2}×")));
        }
        if active
            && self.phase == Some(RunPhase::Transcoding)
            && let (Some(duration), Some(processed), Some(speed)) = (
                self.duration,
                self.progress.out_time_us,
                self.progress.speed,
            )
            && let Some(remaining) = estimate_remaining(duration, processed, speed)
        {
            metrics.push(("ETA", components::format_duration(remaining)));
        }
        if (active || self.task_total == 1)
            && let Some(bytes) = self.progress.total_size
        {
            metrics.push(("Output size", components::format_size(bytes)));
        }
        let mut details = Vec::new();
        if active && let Some(processed) = self.progress.out_time_us {
            let processed = Duration::from_micros(processed.max(0) as u64);
            let value = match self.duration {
                Some(duration) => format!(
                    "Processed {} / {}",
                    components::format_duration(processed),
                    components::format_duration(duration)
                ),
                None => format!("Processed {}", components::format_duration(processed)),
            };
            details.push(value);
        }
        if active && let Some(frame) = self.progress.frame {
            details.push(format!("Frame {frame}"));
        }
        if active && let Some(fps) = self.progress.fps {
            details.push(format!("{fps:.1} fps"));
        }
        if active && let Some(duplicates) = self.progress.dup_frames {
            details.push(format!("{duplicates} duplicate frames"));
        }
        if active && let Some(dropped) = self.progress.drop_frames {
            details.push(format!("{dropped} dropped frames"));
        }
        let headline = div().text_lg().font_medium().child(headline);
        #[cfg(test)]
        let headline = headline.id("transcode-summary-headline").test_support();
        div()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().flex().child(tag.small().outline().child(state)))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(headline)
                            .when_some(current_file, |this, file| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.muted_foreground)
                                        .truncate()
                                        .child(file),
                                )
                            }),
                    ),
            )
            .when(active, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            ProgressBar::new("transcode-progress")
                                .loading(progress.is_none())
                                .value(progress.unwrap_or_default())
                                .accessibility_label("Transcode progress")
                                .flex_1(),
                        )
                        .when_some(progress, |this, percent| {
                            this.child(div().font_medium().child(format!("{percent:.0}%")))
                        }),
                )
            })
            .when(!metrics.is_empty(), |this| {
                this.child(
                    div()
                        .border_t_1()
                        .border_color(theme.border)
                        .pt_4()
                        .flex()
                        .flex_wrap()
                        .gap_5()
                        .children(metrics.into_iter().map(|(label, value)| {
                            div()
                                .min_w_24()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(label),
                                )
                                .child(div().font_medium().child(value))
                        })),
                )
            })
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{}{} completed · {} failed · {} skipped",
                        if self.task_index > 0 {
                            format!("Task {} of {} · ", self.task_index, self.task_total)
                        } else {
                            String::new()
                        },
                        self.succeeded,
                        self.failed,
                        self.skipped
                    )),
            )
            .when(!details.is_empty(), |this| {
                this.child(div().flex().flex_wrap().gap_x_4().gap_y_1().children(
                    details.into_iter().map(|detail| {
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(detail)
                    }),
                ))
            })
            .when_some(self.notice.clone(), |this, (level, notice)| {
                let color = match level {
                    MessageLevel::Error => theme.danger,
                    MessageLevel::Warning => theme.warning,
                    MessageLevel::Debug | MessageLevel::Info => theme.border,
                };
                this.child(
                    div()
                        .border_l_2()
                        .border_color(color)
                        .bg(theme.muted)
                        .px_3()
                        .py_2()
                        .text_sm()
                        .child(notice),
                )
            })
            .when_some(self.completed_output.clone(), |this, path| {
                this.child(
                    Button::new("reveal-output")
                        .outline()
                        .label("Reveal output")
                        .on_click(move |_, _, cx| cx.reveal_path(&path)),
                )
            })
    }

    fn render_task_outcomes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let results_len = self.results.len();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().font_medium().child("Task outcomes"))
            .child(
                div()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .children(self.results.iter().enumerate().map(|(index, result)| {
                        let (label, tag) = match result.status {
                            ResultStatus::Success => ("Completed", Tag::success()),
                            ResultStatus::Failure => ("Failed", Tag::danger()),
                            ResultStatus::Cancelled => ("Cancelled", Tag::secondary()),
                            ResultStatus::Skipped => ("Skipped", Tag::warning()),
                        };
                        let file = Path::new(&result.name)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| result.name.clone());
                        let show_path = file != result.name;
                        div()
                            .p_4()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .when(index + 1 < results_len, |this| {
                                this.border_b_1().border_color(theme.border)
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_start()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(div().font_medium().truncate().child(file))
                                            .when(show_path, |this| {
                                                this.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(theme.muted_foreground)
                                                        .truncate()
                                                        .child(result.name.clone()),
                                                )
                                            }),
                                    )
                                    .child(tag.small().outline().child(label)),
                            )
                            .when_some(result.error.clone(), |this, error| {
                                this.child(div().text_sm().text_color(theme.danger).child(error))
                            })
                    })),
            )
    }

    fn render_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_lg().font_medium().child("Results"))
            .when(matches!(self.stage, Stage::Idle), |this| {
                this.child(components::empty_results(cx))
            })
            .when(!matches!(self.stage, Stage::Idle), |this| {
                this.child(self.render_run_summary(cx))
            })
            .when(!self.results.is_empty(), |this| {
                this.child(self.render_task_outcomes(cx))
            })
    }

    fn render_work_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let results = div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_y_scrollbar()
            .child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(self.source.clone())
                    .child(self.render_results(cx)),
            );
        components::work_and_messages(
            "transcode-work-and-messages",
            results.into_any_element(),
            self.messages.clone(),
            cx,
        )
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = matches!(self.stage, Stage::Running | Stage::Cancelling);
        let ready =
            self.source.read(cx).valid_path().is_some() && self.settings.read(cx).has_output(cx);
        let footer = div()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(div().flex().justify_end().child(if running {
                Button::new("cancel-transcode")
                    .danger()
                    .label(if matches!(self.stage, Stage::Cancelling) {
                        "Cancelling…"
                    } else {
                        "Cancel"
                    })
                    .disabled(matches!(self.stage, Stage::Cancelling))
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
            } else {
                Button::new("start-transcode")
                    .primary()
                    .label("Start transcode")
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
            }));
        components::inspector(
            self.settings.clone().into_any_element(),
            footer.into_any_element(),
            cx,
        )
    }
}

fn estimate_remaining(duration: Duration, processed_us: i64, speed: f64) -> Option<Duration> {
    if speed <= 0.0 {
        return None;
    }
    let remaining = (duration.as_secs_f64() - processed_us.max(0) as f64 / 1_000_000.0).max(0.0);
    Duration::try_from_secs_f64(remaining / speed).ok()
}

impl Drop for TranscodePage {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
    }
}

impl Render for TranscodePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let work_area = self.render_work_area(cx).into_any_element();
        let inspector = self.render_inspector(cx).into_any_element();
        components::page_layout("transcode-panes", "Transcode", work_area, inspector, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::{Stage, WorkerMessage, components, estimate_remaining};
    use crate::workspace::{Mode, Workspace};
    use gpui_kit::{
        AppContext as _, TestAppContext, component::Root, px, size, test::TestWindowExt as _,
    };
    use std::time::Duration;
    use yog_core::ffmpeg::progress::Progress;
    use yog_runtime::{
        RunStatus,
        event::{RunEvent, RunPhase},
    };

    #[test]
    fn progress_uses_adaptive_video_size_units() {
        assert_eq!(components::format_size(800 * 1_048_576), "800 MiB");
        assert_eq!(components::format_size(1_181_116_006), "1.1 GiB");
    }

    #[test]
    fn processed_time_keeps_hours_instead_of_wrapping_at_sixty_minutes() {
        assert_eq!(
            components::format_duration(Duration::from_secs(3_661)),
            "01:01:01"
        );
    }

    #[test]
    fn eta_uses_processed_media_time_and_encoder_speed() {
        assert_eq!(
            estimate_remaining(Duration::from_secs(120), 30_000_000, 2.0),
            Some(Duration::from_secs(45))
        );
        assert_eq!(estimate_remaining(Duration::from_secs(120), 0, 0.0), None);
    }

    #[gpui_kit::test]
    fn messages_collapse_and_background_transcode_remains_visible(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(820.), px(640.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("messages-filter").is_none());
            let collapsed_top = window.find("toggle-messages").bounds().top();

            window.click("toggle-messages", cx);
            assert!(window.find("messages-filter").visible());
            assert!(window.find("toggle-messages").bounds().top() < collapsed_top);
            window.click("toggle-messages", cx);
            assert!(window.try_find("messages-filter").is_none());

            let transcode = workspace.read(cx).transcode.clone();
            transcode.update(cx, |transcode, cx| {
                transcode.stage = Stage::Running;
                transcode.phase = Some(RunPhase::Transcoding);
                transcode.task_index = 2;
                transcode.task_total = 8;
                transcode.duration = Some(Duration::from_secs(100));
                transcode.progress.out_time_us = Some(25_000_000);
                cx.notify();
            });
            workspace.update(cx, |workspace, cx| {
                workspace.change_mode(Mode::Predict, window, cx);
            });
            window.render_frame(cx);
            assert!(window.find("view-transcode").visible());
            assert_eq!(
                window.find("view-transcode").label(),
                Some("View transcode: Running, Transcoding · 2/8 · 25%")
            );

            transcode.update(cx, |transcode, cx| {
                transcode.handle_message(
                    WorkerMessage::Event(RunEvent::Progress {
                        duration: Some(Duration::from_secs(100)),
                        progress: Progress {
                            out_time_us: Some(50_000_000),
                            ..Progress::default()
                        },
                    }),
                    window,
                    cx,
                );
            });
        })
        .unwrap();
        cx.run_until_parked();

        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("view-transcode").label(),
                Some("View transcode: Running, Transcoding · 2/8 · 50%")
            );

            window.click("view-transcode", cx);
            assert!(matches!(workspace.read(cx).mode, Mode::Transcode));
            assert!(window.try_find("view-transcode").is_none());

            let transcode = workspace.read(cx).transcode.clone();
            transcode.update(cx, |transcode, cx| {
                transcode.stage = Stage::Finished(RunStatus::Failure);
                cx.notify();
            });
            workspace.update(cx, |workspace, cx| {
                workspace.change_mode(Mode::Emulate, window, cx);
            });
            window.render_frame(cx);
            assert_eq!(
                window.find("view-transcode").label(),
                Some("View transcode: Failed, Transcode")
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn narrow_results_keep_the_headline_readable(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(600.), px(580.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            let transcode = workspace.read(cx).transcode.clone();
            transcode.update(cx, |transcode, cx| {
                transcode.stage = Stage::Finished(RunStatus::Cancelled);
                transcode.current_name = Some("test.mkv".into());
                cx.notify();
            });
            window.render_frame(cx);
            let headline = window.find("transcode-summary-headline");
            assert!(
                headline.bounds().size.width >= px(110.),
                "headline bounds: {:?}",
                headline.bounds()
            );
            assert!(
                headline.bounds().size.height <= px(60.),
                "headline bounds: {:?}",
                headline.bounds()
            );
        })
        .unwrap();
    }
}
