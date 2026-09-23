mod settings;

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme, Disableable, StyledExt as _,
    button::{Button, ButtonVariants},
    progress::Progress as ProgressBar,
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
use super::{components, source::SourcePicker};

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
    results: Vec<TaskResult>,
    _source_observation: Subscription,
    _settings_observation: Subscription,
}

impl TranscodePage {
    pub fn new(source: Entity<SourcePicker>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.new(|cx| TranscodeSettings::new(source.clone(), window, cx));
        let source_observation = cx.observe(&source, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        Self {
            source,
            settings,
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
            results: Vec::new(),
            _source_observation: source_observation,
            _settings_observation: settings_observation,
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Running | Stage::Cancelling) {
            return;
        }
        let request = match self.settings.read(cx).build(cx) {
            Ok(request) => request,
            Err(error) => {
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
        self.results.clear();

        let cancellation = CancellationToken::new();
        self.cancellation = Some(cancellation.clone());
        let (sender, mut receiver) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
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
            _ = sender.send(WorkerMessage::Finished(outcome));
        });
        cx.spawn(async move |this, cx| {
            while let Some(message) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| this.handle_message(message, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
            self.stage = Stage::Cancelling;
            cx.notify();
        }
    }

    fn handle_message(&mut self, message: WorkerMessage, cx: &mut Context<Self>) {
        match message {
            WorkerMessage::Event(event) => match event {
                RunEvent::PhaseChanged(phase) => {
                    self.phase = Some(phase);
                    if phase != RunPhase::Transcoding {
                        self.duration = None;
                        self.progress = Progress::default();
                    }
                }
                RunEvent::BatchDiscovered { total, skipped } => {
                    self.task_total = total;
                    self.skipped = skipped;
                }
                RunEvent::InputSkipped { input, error } => {
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
                    self.duration = None;
                    self.progress = Progress::default();
                }
                RunEvent::Progress { duration, progress } => {
                    self.duration = duration;
                    self.progress = progress;
                }
                RunEvent::VmafFinished { outcome, .. } => match outcome {
                    VmafOutcome::Scored(score) => {
                        self.notice = Some(format!("VMAF {score:.2}"));
                    }
                    VmafOutcome::Failed(error) => {
                        self.notice = Some(format!("VMAF failed: {error}"));
                    }
                    VmafOutcome::Cancelled => {
                        self.notice = Some("VMAF cancelled".into());
                    }
                },
                RunEvent::Warning { message } | RunEvent::Error { message } => {
                    self.notice = Some(message);
                }
                RunEvent::TaskFinished {
                    input,
                    total,
                    status,
                    error,
                    ..
                } => {
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
                self.cancellation = None;
                match outcome {
                    RunOutcome::Failed(error) => self.error = Some(format!("{error:#}")),
                    RunOutcome::Batch(batch) => {
                        self.task_total = batch.total;
                        self.succeeded = batch.succeeded;
                        self.failed = batch.failures.len();
                    }
                    RunOutcome::Completed | RunOutcome::Cancelled => {}
                }
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
        Some((elapsed.max(0) as f64 / duration.as_micros() as f64 * 100.) as f32)
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
        let progress = self.progress_percent();

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
                        .when_some(self.progress.speed, |this, speed| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{speed:.2}× speed")),
                            )
                        })
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
        div()
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
                    .label(if matches!(self.stage, Stage::Cancelling) {
                        "Cancelling…"
                    } else {
                        "Cancel"
                    })
                    .disabled(matches!(self.stage, Stage::Cancelling))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
            } else {
                Button::new("start-transcode")
                    .primary()
                    .label("Start transcode")
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx)))
            }));
        components::inspector(
            self.settings.clone().into_any_element(),
            footer.into_any_element(),
            cx,
        )
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
