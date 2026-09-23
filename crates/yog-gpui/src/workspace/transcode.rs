mod settings;

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme, Disableable, StyledExt as _,
    button::{Button, ButtonVariants},
    progress::Progress as ProgressBar,
    resizable::{resizable_panel, v_resizable},
    scroll::ScrollableElement,
};
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use yog_core::ffmpeg::progress::Progress;
use yog_runtime::{
    RunOutcome, RunStatus,
    event::{RunEvent, RunPhase, TaskStatus, VmafOutcome},
};

use self::settings::TranscodeSettings;
use super::{
    components,
    messages::{MessageLevel, MessageSource, Messages, NewMessage},
    source::SourcePicker,
};
use crate::logging;

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

enum WorkerMessage {
    Event(RunEvent),
    Finished(RunOutcome),
}

pub struct TranscodePage {
    source: Entity<SourcePicker>,
    settings: Entity<TranscodeSettings>,
    messages: Entity<Messages>,
    stage: Stage,
    cancellation: Option<CancellationToken>,
    error: Option<String>,
    notice: Option<String>,
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
}

impl TranscodePage {
    pub fn new(source: Entity<SourcePicker>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.new(|cx| TranscodeSettings::new(source.clone(), window, cx));
        let messages = cx.new(|cx| Messages::new(window, cx));
        let source_observation = cx.observe(&source, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
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
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let (log_sender, mut log_receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            let capture = logging::capture(log_sender);
            let outcome = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(yog_runtime::run_with_events(
                    request.command,
                    request.options,
                    request.config,
                    cancellation,
                    |event| {
                        _ = sender.send(WorkerMessage::Event(event));
                    },
                )),
                Err(error) => RunOutcome::Failed(error.into()),
            };
            drop(capture);
            _ = sender.send(WorkerMessage::Finished(outcome));
        });
        cx.spawn(async move |this, cx| {
            let mut events_open = true;
            let mut logs_open = true;
            while events_open || logs_open {
                tokio::select! {
                    message = receiver.recv(), if events_open => match message {
                        Some(message) => {
                            if matches!(message, WorkerMessage::Finished(_)) {
                                // The worker closes the log sink before sending Finished. Drain
                                // its remaining records before another run can clear the panel.
                                while let Ok(log) = log_receiver.try_recv() {
                                    if this.update_in(cx, |this, window, cx| {
                                        this.messages.update(cx, |messages, cx| messages.push(log, window, cx));
                                    }).is_err() {
                                        return;
                                    }
                                }
                                logs_open = false;
                                if this.update_in(cx, |this, window, cx| {
                                    this.messages.update(cx, |messages, cx| messages.finish(window, cx));
                                }).is_err() {
                                    return;
                                }
                            }
                            if this.update_in(cx, |this, window, cx| this.handle_message(message, window, cx)).is_err() {
                                break;
                            }
                        }
                        None => events_open = false,
                    },
                    message = log_receiver.recv(), if logs_open => match message {
                        Some(message) => {
                            if this.update_in(cx, |this, window, cx| {
                                this.messages.update(cx, |messages, cx| messages.push(message, window, cx));
                            }).is_err() {
                                break;
                            }
                        }
                        None => {
                            logs_open = false;
                            _ = this.update_in(cx, |this, window, cx| {
                                this.messages.update(cx, |messages, cx| messages.finish(window, cx));
                            });
                        }
                    },
                }
            }
        })
        .detach();
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
                        self.notice = Some(format!("VMAF {score:.2}"));
                        self.message(
                            MessageLevel::Info,
                            format!("VMAF {score:.2}: {}", output.display()),
                            window,
                            cx,
                        );
                    }
                    VmafOutcome::Failed(error) => {
                        self.notice = Some(format!("VMAF failed: {error}"));
                    }
                    VmafOutcome::Cancelled => {
                        self.notice = Some("VMAF cancelled".into());
                    }
                },
                RunEvent::Warning { message } => {
                    self.message(MessageLevel::Warning, message.clone(), window, cx);
                    self.notice = Some(message);
                }
                RunEvent::Error { message } => {
                    self.message(MessageLevel::Error, message.clone(), window, cx);
                    self.notice = Some(message);
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
        cx.notify();
    }

    fn phase_label(&self) -> &'static str {
        match self.phase {
            Some(RunPhase::Discovering) => "Discovering files",
            Some(RunPhase::Probing) => "Probing media",
            Some(RunPhase::Planning) => "Preparing transcode",
            Some(RunPhase::Predicting) => "Predicting",
            Some(RunPhase::Transcoding) => "Transcoding",
            Some(RunPhase::Verifying) => "Verifying output",
            Some(RunPhase::Publishing) => "Publishing output",
            Some(RunPhase::CalculatingVmaf) => "Calculating VMAF",
            Some(RunPhase::RenderingChart) => "Rendering chart",
            None => "Starting",
        }
    }

    fn progress_percent(&self) -> Option<f32> {
        let duration = self.duration?;
        let elapsed = self.progress.out_time_us?;
        if duration.is_zero() {
            return None;
        }
        Some((elapsed.max(0) as f64 / duration.as_micros() as f64 * 100.).clamp(0.0, 100.0) as f32)
    }

    fn render_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = match self.stage {
            Stage::Idle => "Ready",
            Stage::Running => self.phase_label(),
            Stage::Cancelling => "Cancelling",
            Stage::Finished(RunStatus::Success) => "Completed",
            Stage::Finished(RunStatus::Failure) => "Failed",
            Stage::Finished(RunStatus::Cancelled) => "Cancelled",
        };
        let active = matches!(self.stage, Stage::Running | Stage::Cancelling);
        let progress = (self.phase == Some(RunPhase::Transcoding))
            .then(|| self.progress_percent())
            .flatten();
        let mut metrics = Vec::new();
        if let Some(elapsed) = self
            .finished_elapsed
            .or_else(|| self.started.map(|time| time.elapsed()))
        {
            metrics.push(format!("Run elapsed {}", format_duration(elapsed)));
        }
        if self.phase == Some(RunPhase::Transcoding) {
            if let Some(percent) = self.progress_percent() {
                metrics.push(format!("{percent:.0}% complete"));
            }
            if let (Some(duration), Some(processed), Some(speed)) = (
                self.duration,
                self.progress.out_time_us,
                self.progress.speed,
            ) && let Some(remaining) = estimate_remaining(duration, processed, speed)
            {
                metrics.push(format!("ETA {}", format_duration(remaining)));
            }
        }
        if let Some(processed) = self.progress.out_time_us {
            let processed = Duration::from_micros(processed.max(0) as u64);
            let value = match self.duration {
                Some(duration) => format!(
                    "Processed {} / {}",
                    format_duration(processed),
                    format_duration(duration)
                ),
                None => format!("Processed {}", format_duration(processed)),
            };
            metrics.push(value);
        }
        if let Some(frame) = self.progress.frame {
            metrics.push(format!("Frame {frame}"));
        }
        if let Some(fps) = self.progress.fps {
            metrics.push(format!("{fps:.1} fps"));
        }
        if let Some(speed) = self.progress.speed {
            metrics.push(format!("{speed:.2}× speed"));
        }
        if let Some(bytes) = self.progress.total_size {
            metrics.push(format!("Output {}", format_size(bytes)));
        }
        if let Some(duplicates) = self.progress.dup_frames {
            metrics.push(format!("Duplicate frames {duplicates}"));
        }
        if let Some(dropped) = self.progress.drop_frames {
            metrics.push(format!("Dropped frames {dropped}"));
        }

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().font_medium().child("Results"))
            .when(matches!(self.stage, Stage::Idle), |this| {
                this.child(components::empty_results(cx))
            })
            .when(!matches!(self.stage, Stage::Idle), |this| {
                this.child(
                    div()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.border)
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(div().font_medium().child(status))
                        .when_some(self.current_name.clone(), |this, name| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(name),
                            )
                        })
                        .when(active, |this| {
                            this.child(
                                ProgressBar::new("transcode-progress")
                                    .loading(progress.is_none())
                                    .value(progress.unwrap_or_default())
                                    .accessibility_label("Transcode progress"),
                            )
                        })
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(format!(
                                    "{} of {} · {} completed · {} failed · {} skipped",
                                    self.task_index,
                                    self.task_total,
                                    self.succeeded,
                                    self.failed,
                                    self.skipped
                                )),
                        )
                        .child(div().flex().flex_wrap().gap_x_4().gap_y_2().children(
                            metrics.into_iter().map(|metric| {
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(metric)
                            }),
                        ))
                        .when_some(self.notice.clone(), |this, notice| {
                            this.child(div().text_sm().child(notice))
                        })
                        .when_some(self.completed_output.clone(), |this, path| {
                            this.child(
                                Button::new("reveal-output")
                                    .ghost()
                                    .label("Reveal output")
                                    .on_click(move |_, _, cx| cx.reveal_path(&path)),
                            )
                        }),
                )
            })
            .children(self.results.iter().map(|result| {
                let (label, color) = match result.status {
                    ResultStatus::Success => ("Completed", theme.success),
                    ResultStatus::Failure => ("Failed", theme.danger),
                    ResultStatus::Cancelled => ("Cancelled", theme.muted_foreground),
                    ResultStatus::Skipped => ("Skipped", theme.warning),
                };
                div()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .gap_3()
                            .child(div().min_w_0().truncate().child(result.name.clone()))
                            .child(div().flex_shrink_0().text_color(color).child(label)),
                    )
                    .when_some(result.error.clone(), |this, error| {
                        this.child(div().text_sm().text_color(theme.danger).child(error))
                    })
            }))
    }

    fn render_work_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rem = cx.theme().font_size;
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
        v_resizable("transcode-work-and-messages")
            .child(resizable_panel().child(results))
            .child(
                resizable_panel()
                    .size(rem * 18.0)
                    .size_range(rem * 12.0..rem * 32.0)
                    .flex_none()
                    .child(self.messages.clone()),
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

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn estimate_remaining(duration: Duration, processed_us: i64, speed: f64) -> Option<Duration> {
    if speed <= 0.0 {
        return None;
    }
    let remaining = (duration.as_secs_f64() - processed_us.max(0) as f64 / 1_000_000.0).max(0.0);
    Duration::try_from_secs_f64(remaining / speed).ok()
}

fn format_size(bytes: u64) -> String {
    let mib = bytes as f64 / 1_048_576.0;
    if mib >= 1_024.0 {
        format!("{:.1} GiB", mib / 1_024.0)
    } else if mib >= 10.0 {
        format!("{mib:.0} MiB")
    } else {
        format!("{mib:.1} MiB")
    }
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
    use super::{estimate_remaining, format_duration, format_size};
    use std::time::Duration;

    #[test]
    fn progress_uses_adaptive_video_size_units() {
        assert_eq!(format_size(800 * 1_048_576), "800 MiB");
        assert_eq!(format_size(1_181_116_006), "1.1 GiB");
    }

    #[test]
    fn processed_time_keeps_hours_instead_of_wrapping_at_sixty_minutes() {
        assert_eq!(format_duration(Duration::from_secs(3_661)), "01:01:01");
    }

    #[test]
    fn eta_uses_processed_media_time_and_encoder_speed() {
        assert_eq!(
            estimate_remaining(Duration::from_secs(120), 30_000_000, 2.0),
            Some(Duration::from_secs(45))
        );
        assert_eq!(estimate_remaining(Duration::from_secs(120), 0, 0.0), None);
    }
}
