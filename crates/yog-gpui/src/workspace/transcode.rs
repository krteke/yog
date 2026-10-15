mod recovery;
mod settings;
mod skip;

use std::{
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants},
    progress::Progress as ProgressBar,
    tag::Tag,
};
#[cfg(test)]
use gpui_kit::test::TestSupportExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ListAlignment, ListState, ParentElement as _, Render, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use yog_core::ffmpeg::progress::Progress;
use yog_runtime::control::RunControl;
use yog_runtime::{
    RunOutcome, RunStatus,
    event::{
        RecoveryInfo, RunEvent, RunPhase, SkipReason, TaskResult as RuntimeTaskResult, TaskStatus,
        VmafOutcome,
    },
};

use self::{
    recovery::{RecoveryAnswered, RecoveryDialog},
    settings::TranscodeSettings,
    skip::{SkipDialog, SkipRequested},
};
use super::{
    components,
    execution::{self, Activity, ActivityChanged, WorkerMessage},
    messages::{ExpansionChanged, MessageLevel, MessageSource, Messages, NewMessage},
    result_filter::ResultFilter,
    settings::SettingsPanel,
    source::SourcePicker,
};

enum Stage {
    Idle,
    Running,
    Skipping,
    Cancelling,
    Finished(RunStatus),
}

struct TaskResult {
    name: String,
    result: RuntimeTaskResult,
    recovery: Option<RecoveryInfo>,
}

pub struct TranscodePage {
    source: Entity<SourcePicker>,
    settings: Entity<TranscodeSettings>,
    messages: Entity<Messages>,
    recovery: Entity<RecoveryDialog>,
    skip: Entity<SkipDialog>,
    stage: Stage,
    control: Option<RunControl>,
    error: Option<String>,
    notice: Option<(MessageLevel, String)>,
    phase: Option<RunPhase>,
    current_name: Option<String>,
    current_output: Option<PathBuf>,
    completed_output: Option<PathBuf>,
    task_index: usize,
    task_total: usize,
    batch: bool,
    active_task: Option<usize>,
    succeeded: usize,
    failed: usize,
    skipped: usize,
    reused: usize,
    forced: usize,
    state_path: Option<PathBuf>,
    duration: Option<std::time::Duration>,
    progress: Progress,
    started: Option<Instant>,
    finished_elapsed: Option<Duration>,
    results: Vec<TaskResult>,
    work_list: ListState,
    result_filter: Entity<ResultFilter>,
    _filter_observation: Subscription,
    _source_observation: Subscription,
    _settings_observation: Subscription,
    _messages_subscription: Subscription,
    _recovery_subscription: Subscription,
    _skip_subscription: Subscription,
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
        let recovery = cx.new(|_| RecoveryDialog::new());
        let skip = cx.new(|cx| SkipDialog::new(window, cx));
        let skip_subscription = cx.subscribe_in(
            &skip,
            window,
            |page, _, request: &SkipRequested, window, cx| {
                page.skip_tasks(request.index, request.count, window, cx);
            },
        );
        let recovery_subscription = cx.subscribe(&recovery, |_, _, _: &RecoveryAnswered, cx| {
            cx.emit(ActivityChanged);
            cx.notify();
        });
        let source_observation = cx.observe(&source, |_, _, cx| cx.notify());
        let settings_observation = cx.observe(&settings, |_, _, cx| cx.notify());
        let messages_subscription =
            cx.subscribe(&messages, |_, _, _: &ExpansionChanged, cx| cx.notify());
        let work_list = ListState::new(1, ListAlignment::Top, px(300.));
        let result_filter =
            cx.new(|cx| ResultFilter::new(true, false, work_list.clone(), window, cx));
        let filter_observation = cx.observe(&result_filter, |_, _, cx| cx.notify());
        Self {
            source,
            settings,
            messages,
            recovery,
            skip,
            stage: Stage::Idle,
            control: None,
            error: None,
            notice: None,
            phase: None,
            current_name: None,
            current_output: None,
            completed_output: None,
            task_index: 0,
            task_total: 1,
            batch: false,
            active_task: None,
            succeeded: 0,
            failed: 0,
            skipped: 0,
            reused: 0,
            forced: 0,
            state_path: None,
            duration: None,
            progress: Progress::default(),
            started: None,
            finished_elapsed: None,
            results: Vec::new(),
            work_list,
            result_filter,
            _filter_observation: filter_observation,
            _source_observation: source_observation,
            _settings_observation: settings_observation,
            _messages_subscription: messages_subscription,
            _recovery_subscription: recovery_subscription,
            _skip_subscription: skip_subscription,
        }
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.stage,
            Stage::Running | Stage::Skipping | Stage::Cancelling
        ) {
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
        self.batch = false;
        self.active_task = None;
        self.settings
            .update(cx, |settings, cx| settings.set_running(true, cx));
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
        self.reused = 0;
        self.forced = 0;
        self.state_path = None;
        self.duration = None;
        self.progress = Progress::default();
        self.started = Some(Instant::now());
        self.finished_elapsed = None;
        self.results.clear();
        self.result_filter
            .update(cx, |filter, cx| filter.reset(window, cx));
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

        let control = RunControl::new();
        self.control = Some(control.clone());
        let mut receiver = execution::spawn(request, control);
        cx.spawn(async move |this, cx| {
            while let Some(batch) = receiver.recv(cx.background_executor()).await {
                if this
                    .update_in(cx, |this, window, cx| {
                        for message in batch {
                            this.handle_message(message, window, cx);
                        }
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
        if let Some(control) = &self.control {
            control.cancel();
            self.skip.update(cx, |skip, cx| skip.clear(window, cx));
            self.recovery
                .update(cx, |recovery, cx| recovery.clear(window, cx));
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

    fn skip_index(&self) -> Option<usize> {
        if self.batch && matches!(self.stage, Stage::Running) {
            self.active_task
        } else {
            None
        }
    }

    fn skip_tasks(
        &mut self,
        index: usize,
        count: NonZeroUsize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .control
            .as_ref()
            .is_some_and(|control| control.skip_tasks(index, count))
        {
            self.stage = Stage::Skipping;
            self.message(
                MessageLevel::Info,
                if count.get() == 1 {
                    "Skipping current task".into()
                } else {
                    format!("Skipping {count} tasks, including the current task")
                },
                window,
                cx,
            );
        } else {
            self.notice = Some((
                MessageLevel::Info,
                "Task is no longer available to skip".into(),
            ));
        }
        cx.emit(ActivityChanged);
        cx.notify();
    }

    fn open_skip_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.skip_index() {
            let name = self.current_name.clone().expect("active task has an input");
            let vmaf = self.phase == Some(RunPhase::CalculatingVmaf);
            self.skip
                .update(cx, |skip, cx| skip.open(index, name, vmaf, window, cx));
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
                return;
            }
            WorkerMessage::Event(event) => match event {
                RunEvent::RecoveryDecisionRequired(request) => {
                    self.state_path = Some(request.state_path.clone());
                    self.message(
                        MessageLevel::Warning,
                        format!("Waiting for confirmation: {}", request.title()),
                        window,
                        cx,
                    );
                    self.recovery
                        .update(cx, |recovery, cx| recovery.show(request, window, cx));
                }
                RunEvent::PhaseChanged(phase) => {
                    self.phase = Some(phase);
                    self.message(MessageLevel::Info, self.phase_label().into(), window, cx);
                }
                RunEvent::BatchDiscovered { total } => {
                    self.batch = true;
                    self.task_total = total;
                    self.message(
                        MessageLevel::Info,
                        format!("Found {total} input files"),
                        window,
                        cx,
                    );
                }
                RunEvent::TaskStarted {
                    index,
                    total,
                    input,
                    output,
                } => {
                    self.task_index = index;
                    self.active_task = Some(index);
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
                    VmafOutcome::ManuallySkipped => {
                        self.notice = Some((MessageLevel::Info, "VMAF manually skipped".into()));
                        self.message(
                            MessageLevel::Info,
                            format!("VMAF manually skipped: {}", output.display()),
                            window,
                            cx,
                        );
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
                RunEvent::RecoveryPrepared {
                    path,
                    reused,
                    forced,
                } => {
                    self.state_path = Some(path.clone());
                    self.message(
                        MessageLevel::Info,
                        format!(
                            "Recovery: {} · {reused} reusable · {forced} forced",
                            path.display()
                        ),
                        window,
                        cx,
                    );
                }
                RunEvent::TaskFinished {
                    input,
                    index,
                    total,
                    result,
                    recovery,
                    ..
                } => {
                    let unstarted = self.active_task.take().is_none();
                    if unstarted
                        && matches!(
                            result,
                            RuntimeTaskResult::Skipped {
                                reason: SkipReason::Manual,
                                ..
                            }
                        )
                    {
                        self.current_name = Some(input.display().to_string());
                        self.current_output = None;
                        self.phase = None;
                        self.duration = None;
                        self.progress = Progress::default();
                    }
                    self.skip.update(cx, |skip, cx| skip.invalidate(index, cx));
                    if matches!(self.stage, Stage::Skipping) {
                        self.stage = Stage::Running;
                    }
                    self.task_index = index;
                    self.task_total = total;
                    self.progress.finished = true;
                    if recovery.as_ref().is_some_and(|info| info.forced) {
                        self.forced += 1;
                    }
                    let reused = matches!(
                        result,
                        RuntimeTaskResult::Skipped {
                            reason: SkipReason::AlreadyCompleted,
                            ..
                        }
                    );
                    let level = match result.status() {
                        TaskStatus::Success => {
                            self.succeeded += 1;
                            MessageLevel::Info
                        }
                        TaskStatus::Failure => {
                            self.failed += 1;
                            MessageLevel::Error
                        }
                        TaskStatus::Skipped => {
                            if reused {
                                self.reused += 1;
                                MessageLevel::Info
                            } else {
                                self.skipped += 1;
                                if matches!(
                                    result,
                                    RuntimeTaskResult::Skipped {
                                        reason: SkipReason::Manual,
                                        ..
                                    }
                                ) {
                                    MessageLevel::Info
                                } else {
                                    MessageLevel::Warning
                                }
                            }
                        }
                        TaskStatus::Cancelled => MessageLevel::Warning,
                    };
                    self.message(
                        level,
                        format!(
                            "{result}: {}{}",
                            input.display(),
                            result
                                .message()
                                .map_or(String::new(), |message| format!(": {message}"))
                        ),
                        window,
                        cx,
                    );
                    if (result.status() == TaskStatus::Success || reused) && total == 1 {
                        self.completed_output = recovery
                            .as_ref()
                            .and_then(|info| info.output.clone())
                            .or_else(|| self.current_output.clone());
                    }
                    self.result_filter
                        .update(cx, |filter, cx| filter.append(&result, window, cx));
                    self.results.push(TaskResult {
                        name: input.display().to_string(),
                        result,
                        recovery,
                    });
                }
                _ => {}
            },
            WorkerMessage::Finished(outcome) => {
                self.active_task = None;
                self.skip.update(cx, |skip, cx| skip.clear(window, cx));
                self.recovery
                    .update(cx, |recovery, cx| recovery.clear(window, cx));
                self.messages
                    .update(cx, |messages, cx| messages.finish(window, cx));
                self.stage = Stage::Finished(outcome.status());
                self.settings
                    .update(cx, |settings, cx| settings.set_running(false, cx));
                self.finished_elapsed = self.started.map(|started| started.elapsed());
                self.control = None;
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
                        self.skipped = batch.skipped;
                        self.reused = batch.reused;
                        self.forced = batch.forced;
                        batch.state_error.or(batch.recovery_error).map(|error| {
                            let detail = format!("{error:#}");
                            self.error = Some(detail.clone());
                            detail
                        })
                    }
                    RunOutcome::Completed | RunOutcome::Cancelled => None,
                };
                let (level, label) = match self.stage {
                    Stage::Finished(RunStatus::Success) => (MessageLevel::Info, "Run completed"),
                    Stage::Finished(RunStatus::Failure) => (MessageLevel::Error, "Run failed"),
                    Stage::Finished(RunStatus::Cancelled) => {
                        (MessageLevel::Warning, "Run cancelled")
                    }
                    Stage::Idle | Stage::Running | Stage::Skipping | Stage::Cancelling => {
                        unreachable!()
                    }
                };
                self.message(
                    level,
                    failure.map_or_else(|| label.to_owned(), |error| format!("{label}: {error}")),
                    window,
                    cx,
                );
            }
        }
        self.work_list.remeasure_items(0..1);
        cx.emit(ActivityChanged);
        cx.notify();
    }

    fn phase_label(&self) -> &'static str {
        if self.batch && self.active_task.is_none() && self.task_index > 0 {
            "Processing batch"
        } else if self.phase == Some(RunPhase::Planning) {
            "Preparing transcode"
        } else {
            execution::phase_label(self.phase)
        }
    }

    pub fn activity(&self, cx: &gpui_kit::App) -> Option<Activity> {
        match self.stage {
            Stage::Idle => None,
            Stage::Running => {
                if self.recovery.read(cx).is_waiting() {
                    return Some(Activity::Waiting("Recovery decision required".into()));
                }
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
            Stage::Skipping => Some(Activity::Running("Skipping current task".into())),
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
            Stage::Running if self.recovery.read(cx).is_waiting() => {
                ("Waiting for confirmation", "Waiting", Tag::warning())
            }
            Stage::Running => (self.phase_label(), "Running", Tag::info()),
            Stage::Cancelling => ("Cancelling transcode", "Cancelling", Tag::warning()),
            Stage::Skipping => ("Skipping current task", "Skipping", Tag::warning()),
            Stage::Finished(RunStatus::Success) => {
                ("Transcode complete", "Completed", Tag::success())
            }
            Stage::Finished(RunStatus::Failure) => ("Transcode failed", "Failed", Tag::danger()),
            Stage::Finished(RunStatus::Cancelled) => {
                ("Transcode cancelled", "Cancelled", Tag::secondary())
            }
        };
        let active = matches!(
            self.stage,
            Stage::Running | Stage::Skipping | Stage::Cancelling
        );
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
                        "{}{} completed · {} failed · {} skipped · {} reused · {} forced",
                        if self.task_index > 0 {
                            format!("Task {} of {} · ", self.task_index, self.task_total)
                        } else {
                            String::new()
                        },
                        self.succeeded,
                        self.failed,
                        self.skipped,
                        self.reused,
                        self.forced
                    )),
            )
            .when_some(self.state_path.clone(), |this, path| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(format!("State: {}", path.display())),
                )
            })
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

    fn render_task_outcome(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let result = &self.results[index];
        let (label, tag) = match result.result.status() {
            TaskStatus::Success => ("Completed", Tag::success()),
            TaskStatus::Failure => ("Failed", Tag::danger()),
            TaskStatus::Cancelled => ("Cancelled", Tag::secondary()),
            TaskStatus::Skipped => ("Skipped", Tag::warning()),
        };
        let file = Path::new(&result.name)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| result.name.clone());
        let show_path = file != result.name;
        let row = div()
            .min_w_0()
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
            .when_some(result.result.message().map(str::to_owned), |this, error| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(if result.result.status() == TaskStatus::Failure {
                            theme.danger
                        } else {
                            theme.muted_foreground
                        })
                        .child(format!("{}: {error}", result.result)),
                )
            });
        let row = row.when_some(result.recovery.as_ref(), |this, recovery| {
            this.child(div().text_xs().text_color(theme.muted_foreground).child(
                if recovery.forced {
                    "Forced recovery"
                } else {
                    "Recovered from state"
                },
            ))
            .when_some(recovery.output.as_ref(), |this, output| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(format!("Recorded output: {}", output.display())),
                )
            })
        });
        let row = row.id(format!("transcode-result-{}", result.name));
        #[cfg(test)]
        let row = row.test_support();
        row
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
                    .child(self.result_filter.clone())
            })
            .when(!self.results.is_empty(), |this| {
                this.child(div().font_medium().child("Task outcomes"))
            })
    }

    fn render_work_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let results = components::virtual_list(
            "transcode-results",
            self.work_list.clone(),
            cx.theme().font_size * 10.,
            move |index, _, cx| {
                view.update(cx, |page, cx| {
                    if index == 0 {
                        div()
                            .p_6()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .child(page.source.clone())
                            .child(page.render_results(cx))
                            .into_any_element()
                    } else {
                        div()
                            .px_6()
                            .pb_4()
                            .child(page.render_task_outcome(
                                page.result_filter.read(cx).source_index(index - 1),
                                cx,
                            ))
                            .into_any_element()
                    }
                })
            },
        );
        components::work_and_messages(
            "transcode-work-and-messages",
            results.into_any_element(),
            self.messages.clone(),
            cx,
        )
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = matches!(
            self.stage,
            Stage::Running | Stage::Skipping | Stage::Cancelling
        );
        let ready =
            self.source.read(cx).valid_path().is_some() && self.settings.read(cx).has_output(cx);
        let footer = div()
            .flex()
            .flex_col()
            .gap_2()
            .when(self.recovery.read(cx).is_waiting(), |this| {
                this.child(
                    Button::new("review-recovery")
                        .label("Review recovery…")
                        .on_click(cx.listener(|page, _, window, cx| {
                            page.recovery
                                .update(cx, |recovery, cx| recovery.open(window, cx));
                        })),
                )
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .when(self.batch && running, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("skip-current")
                                .label(if self.phase == Some(RunPhase::CalculatingVmaf) {
                                    "Skip VMAF"
                                } else {
                                    "Skip current"
                                })
                                .disabled(self.skip_index().is_none())
                                .on_click(cx.listener(|page, _, window, cx| {
                                    if let Some(index) = page.skip_index() {
                                        page.skip_tasks(
                                            index,
                                            NonZeroUsize::new(1).unwrap(),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                        )
                        .child(
                            Button::new("skip-tasks")
                                .label("Skip tasks…")
                                .disabled(self.skip_index().is_none())
                                .on_click(cx.listener(|page, _, window, cx| {
                                    page.open_skip_dialog(window, cx)
                                })),
                        ),
                )
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
        if let Some(control) = &self.control {
            control.cancel();
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

    #[gpui_kit::test]
    #[cfg(unix)]
    fn submitting_skip_count_controls_the_running_batch(cx: &mut TestAppContext) {
        use std::{fs, os::unix::fs::PermissionsExt, sync::mpsc};
        use yog_core::ffmpeg::plan::TranscodeRequest;
        use yog_runtime::{Command, Config, Operation, Options, RunOutcome, control::RunControl};

        struct RunningBatch {
            control: RunControl,
            worker: Option<std::thread::JoinHandle<RunOutcome>>,
        }
        impl Drop for RunningBatch {
            fn drop(&mut self) {
                self.control.cancel();
                if let Some(worker) = self.worker.take() {
                    worker.join().unwrap();
                }
            }
        }

        let fixture = tempfile::tempdir().unwrap();
        let input = fixture.path().join("input");
        let output = fixture.path().join("output");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("first.mkv"), b"video").unwrap();
        fs::write(input.join("second.mkv"), b"video").unwrap();
        let probe = fixture.path().join("ffprobe");
        fs::write(&probe, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
        let control = RunControl::new();
        let (events, receiver) = mpsc::channel();
        let backend_control = control.clone();
        let worker = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(yog_runtime::run_with_events(
                    Command {
                        request: TranscodeRequest::new(input, output),
                        operation: Operation::Transcode,
                        recursive: true,
                    },
                    Options {
                        ffprobe: probe,
                        timeout: Some(Duration::from_secs(10)),
                        terminal_output: false,
                        ..Options::default()
                    },
                    Config::default(),
                    backend_control,
                    move |event| {
                        let _ = events.send(event);
                    },
                ))
        });
        let mut running = RunningBatch {
            control: control.clone(),
            worker: Some(worker),
        };
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(1100.), px(850.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            workspace.read(cx).transcode.clone().update(cx, |page, cx| {
                page.stage = Stage::Running;
                page.control = Some(control.clone());
                loop {
                    let event = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
                    let started = matches!(event, RunEvent::TaskStarted { .. });
                    page.handle_message(WorkerMessage::Event(event), window, cx);
                    if started {
                        break;
                    }
                }
            });
            window.render_frame(cx);
            window.click("skip-tasks", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("skip-count", cx);
            window.press("secondary-a", cx);
            window.input("2", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(
                matches!(workspace.read(cx).transcode.read(cx).stage, Stage::Skipping),
                "notice: {:?}, count: {:?}",
                workspace.read(cx).transcode.read(cx).notice,
                workspace
                    .read(cx)
                    .transcode
                    .read(cx)
                    .skip
                    .read(cx)
                    .count(cx)
            );
            assert!(!control.is_cancelled());
        });
        let outcome = running.worker.take().unwrap().join().unwrap();
        assert_eq!(outcome.status(), RunStatus::Success, "{outcome:?}");
        let RunOutcome::Batch(batch) = outcome else {
            panic!("expected batch")
        };
        assert_eq!(batch.manual_skipped, 2);
        assert!(batch.failures.is_empty());
    }

    #[gpui_kit::test]
    fn skip_count_dialog_is_bound_to_the_displayed_task(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(1100.), px(850.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.transcode.update(cx, |page, cx| {
                    page.stage = Stage::Running;
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::BatchDiscovered { total: 4 }),
                        window,
                        cx,
                    );
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::TaskStarted {
                            index: 2,
                            total: 4,
                            input: "/in/current.mkv".into(),
                            output: Some("/out/current.mkv".into()),
                        }),
                        window,
                        cx,
                    );
                });
            });
            window.render_frame(cx);
            window.click("skip-tasks", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("skip-count", cx);
            window.press("secondary-a", cx);
            window.input("3", cx);
            assert_eq!(
                workspace
                    .read(cx)
                    .transcode
                    .read(cx)
                    .skip
                    .read(cx)
                    .count(cx)
                    .unwrap()
                    .get(),
                3
            );
            workspace.update(cx, |workspace, cx| {
                workspace.transcode.update(cx, |page, cx| {
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::TaskFinished {
                            index: 2,
                            total: 4,
                            input: "/in/current.mkv".into(),
                            result: yog_runtime::event::TaskResult::Success,
                            recovery: None,
                        }),
                        window,
                        cx,
                    );
                })
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("confirm-skip-tasks", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("confirm-skip-tasks").is_some(),
                "stale submission must leave the dialog open"
            );
            window.press("escape", cx);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn unstarted_manual_skip_clears_the_previous_tasks_progress(cx: &mut TestAppContext) {
        use yog_runtime::event::{SkipReason, TaskResult as RuntimeTaskResult};

        cx.update(gpui_kit::init);
        cx.open_window(size(px(1100.), px(850.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            view.update(cx, |workspace, cx| {
                workspace.transcode.update(cx, |page, cx| {
                    page.stage = Stage::Running;
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::TaskStarted {
                            index: 1,
                            total: 3,
                            input: "first.mkv".into(),
                            output: Some("first.mp4".into()),
                        }),
                        window,
                        cx,
                    );
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::TaskFinished {
                            recovery: None,
                            index: 1,
                            total: 3,
                            input: "first.mkv".into(),
                            result: RuntimeTaskResult::Success,
                        }),
                        window,
                        cx,
                    );
                    page.progress.speed = Some(4.0);
                    page.duration = Some(Duration::from_secs(60));
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::TaskFinished {
                            recovery: None,
                            index: 2,
                            total: 3,
                            input: "second.mkv".into(),
                            result: RuntimeTaskResult::Skipped {
                                reason: SkipReason::Manual,
                                message: "Manually skipped".into(),
                            },
                        }),
                        window,
                        cx,
                    );
                    assert_eq!(page.current_name.as_deref(), Some("second.mkv"));
                    assert!(page.current_output.is_none());
                    assert!(page.progress.speed.is_none());
                    assert!(page.duration.is_none());
                });
            });
            Root::new(view, window, cx)
        });
    }

    fn recovery_request() -> (
        yog_runtime::recovery::RecoveryRequest,
        tokio::sync::oneshot::Receiver<Result<yog_runtime::recovery::RecoveryDecision, String>>,
    ) {
        use yog_runtime::recovery::*;
        let (reply, response) = tokio::sync::oneshot::channel();
        (
            RecoveryRequest {
                state_path: "/out/.yog-state.jsonl".into(),
                input: "/in/movie.mkv".into(),
                output: Some("/out/movie.mkv".into()),
                scope: RecoveryScope::Batch,
                differences: vec![RecoveryDifference {
                    kind: RecoveryDifferenceKind::Container,
                    recorded: "mkv".into(),
                    current: "mp4".into(),
                }],
                overwrite_output: false,
                reply,
            },
            response,
        )
    }

    #[gpui_kit::test]
    fn recovery_request_opens_a_dialog_without_answering_it(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(1_100.), px(900.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let (request, mut response) = recovery_request();
        cx.update_window(handle.into(), |_, window, cx| {
            workspace
                .unwrap()
                .read(cx)
                .transcode
                .clone()
                .update(cx, |page, cx| {
                    page.stage = Stage::Running;
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::RecoveryDecisionRequired(request)),
                        window,
                        cx,
                    );
                });
            window.render_frame(cx);
            assert!(
                window.try_find("recovery-force").is_some(),
                "the recovery decision must be visible"
            );
        })
        .unwrap();
        assert!(matches!(
            response.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
    }

    #[gpui_kit::test]
    fn dismissed_recovery_dialog_keeps_global_waiting_state_and_can_be_answered(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(1_100.), px(900.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();
        let (request, mut response) = recovery_request();
        cx.update_window(handle.into(), |_, window, cx| {
            workspace.read(cx).transcode.clone().update(cx, |page, cx| {
                page.stage = Stage::Running;
                page.handle_message(
                    WorkerMessage::Event(RunEvent::RecoveryDecisionRequired(request)),
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.change_mode(Mode::Predict, window, cx)
            });
            window.render_frame(cx);
            assert!(
                window
                    .find("view-transcode")
                    .label()
                    .unwrap()
                    .contains("Waiting")
            );
            window.click("view-transcode", cx);
            window.click("review-recovery", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("recovery-apply-similar", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("recovery-force", cx);
        })
        .unwrap();
        let decision = response.try_recv().unwrap().unwrap();
        assert_eq!(
            decision.action,
            yog_runtime::recovery::RecoveryAction::Force
        );
        assert!(decision.apply_to_similar);
    }

    #[test]
    fn progress_uses_adaptive_video_size_units() {
        assert_eq!(components::format_size(800 * 1_048_576), "800 MiB");
        assert_eq!(components::format_size(1_181_116_006), "1.1 GiB");
    }

    #[gpui_kit::test]
    fn recovered_success_retains_the_recorded_output_and_is_not_an_ordinary_skip(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            let page = workspace.unwrap().read(cx).transcode.clone();
            page.update(cx, |page, cx| {
                page.handle_message(
                    WorkerMessage::Event(RunEvent::TaskFinished {
                        input: "input.mkv".into(),
                        index: 1,
                        total: 1,
                        result: yog_runtime::event::TaskResult::Skipped {
                            reason: yog_runtime::event::SkipReason::AlreadyCompleted,
                            message: "Recovered".into(),
                        },
                        recovery: Some(yog_runtime::event::RecoveryInfo {
                            forced: true,
                            output: Some("old/output.mkv".into()),
                        }),
                    }),
                    window,
                    cx,
                );
                assert_eq!(page.skipped, 0);
                assert_eq!(
                    page.completed_output.as_deref(),
                    Some(std::path::Path::new("old/output.mkv"))
                );
            });
        })
        .unwrap();
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
    fn large_task_results_only_materialize_near_the_viewport(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            let page = workspace.read(cx).transcode.clone();
            page.update(cx, |page, cx| {
                page.stage = Stage::Running;
                for index in 0..10_000 {
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::TaskFinished {
                            recovery: None,
                            input: format!("input-{index}.mkv").into(),
                            index: 1,
                            total: 1,
                            result: if index == 0 {
                                yog_runtime::event::TaskResult::Success
                            } else {
                                yog_runtime::event::TaskResult::Skipped {
                                    reason: yog_runtime::event::SkipReason::NoVideo,
                                    message: "Invalid input".into(),
                                }
                            },
                        }),
                        window,
                        cx,
                    );
                }
            });
            assert_eq!(page.read(cx).skipped, 9_999);
            assert_eq!(page.read(cx).succeeded, 1);
            assert_eq!(
                page.read(cx).results[0].result.status(),
                yog_runtime::event::TaskStatus::Success
            );
            page.read(cx).work_list.scroll_to(gpui_kit::ListOffset {
                item_ix: 1,
                offset_in_item: px(0.),
            });
            let started = std::time::Instant::now();
            window.render_frame(cx);
            let row_count = gpui_kit::base::test_support::snapshots(window)
                .iter()
                .filter(|item| {
                    matches!(
                        item.path().last(),
                        Some(gpui_kit::ElementId::Name(name))
                            if name.starts_with("transcode-result-")
                    )
                })
                .count();
            eprintln!(
                "10,000 task results: {row_count} visible cards in {:?}",
                started.elapsed()
            );
            assert!(
                row_count > 0 && row_count < 30,
                "materialized {row_count} rows"
            );
            assert!(
                window.try_find("transcode-result-input-9999.mkv").is_none(),
                "offscreen task was materialized"
            );
            assert_eq!(page.read(cx).results.len(), 10_000);
            window.scroll(
                "transcode-results",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-10_000_000.))),
                cx,
            );
            window.render_frame(cx);
            assert!(
                window.find("transcode-result-input-9999.mkv").visible(),
                "last result is unreachable"
            );
            assert!(window.try_find("transcode-result-input-0.mkv").is_none());
            page.update(cx, |page, cx| {
                page.handle_message(
                    WorkerMessage::Event(RunEvent::Progress {
                        duration: Some(Duration::from_secs(100)),
                        progress: Progress {
                            out_time_us: Some(10_000_000),
                            ..Progress::default()
                        },
                    }),
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            assert!(
                window.find("transcode-result-input-9999.mkv").visible(),
                "progress reset the reader's position"
            );
        })
        .unwrap();
        cx.simulate_window_resize(handle.into(), size(px(680.), px(600.)));
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.find("transcode-result-input-9999.mkv").visible(),
                "resize lost the last result"
            );
            window.scroll(
                "transcode-results",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(10_000_000.))),
                cx,
            );
            assert!(window.find("open-video").visible());
            assert!(window.try_find("transcode-result-input-9999.mkv").is_none());
        })
        .unwrap();
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
    fn wide_work_area_stretches_source_summary_and_result_cards(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(1_600.), px(1_100.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            let transcode = workspace.read(cx).transcode.clone();
            transcode.update(cx, |page, cx| {
                page.stage = Stage::Running;
                page.phase = Some(RunPhase::Discovering);
                page.handle_message(
                    WorkerMessage::Event(RunEvent::TaskFinished {
                        recovery: None,
                        input: "input.mkv".into(),
                        index: 1,
                        total: 1,
                        result: yog_runtime::event::TaskResult::Skipped {
                            reason: yog_runtime::event::SkipReason::NoVideo,
                            message: "Invalid input".into(),
                        },
                    }),
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
            let viewport = window.find("transcode-results").bounds();
            let open_folder = window.find("open-folder").bounds();
            assert!(
                open_folder.right() >= viewport.right() - px(32.),
                "source did not fill the work area: button {open_folder:?}, viewport {viewport:?}"
            );
            let headline = window.find("transcode-summary-headline").bounds();
            assert!(
                headline.size.width >= viewport.size.width - px(200.),
                "summary did not fill the work area: headline {headline:?}, viewport {viewport:?}"
            );
            let result = window.find("transcode-result-input.mkv").bounds();
            assert!(
                result.size.width >= viewport.size.width - px(64.),
                "result did not fill the work area: card {result:?}, viewport {viewport:?}"
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
