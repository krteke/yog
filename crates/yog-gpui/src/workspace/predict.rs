mod view;

use std::{
    num::NonZeroU64,
    path::PathBuf,
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants},
    form::{Field, Form},
    input::{Input, InputState},
    progress::Progress as ProgressBar,
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tag::Tag,
};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, Hsla, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use tokio_util::sync::CancellationToken;
use yog_core::ffmpeg::{
    encoding::RateControl,
    plan::{TranscodeRequest, VideoAction},
    prediction::Prediction,
};
use yog_runtime::{
    Command, Config, Operation, Options, RunOutcome, RunStatus, Validate,
    event::{RunEvent, RunPhase, TaskStatus},
};

use super::{
    components,
    execution::{self, Activity, ActivityChanged, RunRequest, WorkerMessage},
    messages::{ExpansionChanged, MessageLevel, MessageSource, Messages, NewMessage},
    source::SourcePicker,
    video::{CONTAINERS, Choice, VideoInputs, container, selected},
};

enum Stage {
    Idle,
    Running,
    Cancelling,
    Finished(RunStatus),
}

struct ResultRow {
    path: PathBuf,
    prediction: Option<Prediction>,
    error: Option<String>,
}

pub struct PredictPage {
    source: Entity<SourcePicker>,
    video: Entity<VideoInputs>,
    container: Choice,
    rate: Choice,
    quality: Entity<InputState>,
    bitrate: Entity<InputState>,
    report: Entity<InputState>,
    messages: Entity<Messages>,
    stage: Stage,
    cancellation: Option<CancellationToken>,
    error: Option<String>,
    phase: Option<RunPhase>,
    task_index: usize,
    task_total: usize,
    current_name: Option<PathBuf>,
    completed: usize,
    failed: usize,
    skipped: usize,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    results: Vec<ResultRow>,
    _rate_subscription: Subscription,
    _video_observation: Subscription,
    _messages_subscription: Subscription,
}

impl EventEmitter<ActivityChanged> for PredictPage {}

impl PredictPage {
    pub fn new(source: Entity<SourcePicker>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let video = cx.new(|cx| VideoInputs::new(window, cx));
        let video_observation = cx.observe(&video, |_, _, cx| cx.notify());
        let rate = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec!["Quality", "Bitrate", "Encoder default"]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let container = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    CONTAINERS
                        .iter()
                        .map(|(label, _)| *label)
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let rate_subscription = cx.subscribe_in(
            &rate,
            window,
            |_, _, _: &SelectEvent<SearchableVec<&'static str>>, _, cx| cx.notify(),
        );
        let messages = cx.new(|cx| Messages::new(window, cx));
        let messages_subscription =
            cx.subscribe(&messages, |_, _, _: &ExpansionChanged, cx| cx.notify());
        Self {
            source,
            video,
            container,
            rate,
            quality: cx.new(|cx| InputState::new(window, cx).default_value("28")),
            bitrate: cx.new(|cx| InputState::new(window, cx).placeholder("Bits per second")),
            report: cx.new(|cx| InputState::new(window, cx).placeholder("Optional report path")),
            messages,
            stage: Stage::Idle,
            cancellation: None,
            error: None,
            phase: None,
            task_index: 0,
            task_total: 1,
            current_name: None,
            completed: 0,
            failed: 0,
            skipped: 0,
            started: None,
            elapsed: None,
            results: Vec::new(),
            _rate_subscription: rate_subscription,
            _video_observation: video_observation,
            _messages_subscription: messages_subscription,
        }
    }

    fn build(&self, cx: &mut Context<Self>) -> Result<RunRequest, String> {
        let source = self.source.read(cx);
        let input = source.valid_path().ok_or("Select a video file or folder")?;
        let (decoding, mut encoding) = self.video.read(cx).build(cx);
        let rate = match selected(&self.rate, cx) {
            "Quality" => {
                let value = self.quality.read(cx).value();
                Some(RateControl::Quality(
                    value
                        .trim()
                        .parse::<u8>()
                        .map_err(|_| "Quality must be an integer from 0 to 255")?,
                ))
            }
            "Bitrate" => {
                let value = self.bitrate.read(cx).value();
                Some(RateControl::Bitrate(
                    value
                        .trim()
                        .parse::<NonZeroU64>()
                        .map_err(|_| "Bitrate must be an integer greater than zero")?,
                ))
            }
            "Encoder default" => None,
            _ => unreachable!("rate choice is internal"),
        };
        encoding.set_rate(rate);
        let mut request = TranscodeRequest::new(input, PathBuf::new())
            .with_decoding(decoding)
            .with_video(VideoAction::Encode(encoding));
        if let Some(container) = container(selected(&self.container, cx)) {
            request = request.with_container(container);
        }
        let command = Command {
            request,
            operation: Operation::Predict,
            recursive: source.is_directory(),
        };
        command.validate().map_err(|error| format!("{error:#}"))?;
        let report = self.report.read(cx).value();
        let report = (!report.trim().is_empty()).then(|| PathBuf::from(report.trim()));
        if report
            .as_ref()
            .is_some_and(|report| execution::same_path(report, input))
        {
            return Err("Report path must differ from the source".into());
        }
        let options = Options {
            terminal_output: false,
            report,
            ..Options::default()
        };
        let config = Config::load(None).map_err(|error| format!("{error:#}"))?;
        Ok(RunRequest {
            command,
            options,
            config,
        })
    }

    fn choose_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.source.read(cx);
        let directory = source
            .path()
            .map(|path| {
                if source.is_directory() {
                    path.to_path_buf()
                } else {
                    path.parent().unwrap_or(path).to_path_buf()
                }
            })
            .unwrap_or_else(|| PathBuf::from("."));
        let name = source
            .path()
            .and_then(|path| path.file_stem())
            .and_then(|stem| stem.to_str())
            .unwrap_or("prediction");
        let suggested_name = format!("{name}-prediction.jsonl");
        let response = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn_in(window, async move |this, cx| match response.await {
            Ok(Ok(Some(path))) => {
                _ = this.update_in(cx, |this, window, cx| {
                    this.report.update(cx, |report, cx| {
                        report.set_value(path.to_string_lossy().to_string(), window, cx)
                    });
                    this.error = None;
                    cx.notify();
                });
            }
            Ok(Ok(None)) => {}
            result => {
                _ = this.update_in(cx, |this, _, cx| {
                    this.error = Some(format!("Could not open save dialog: {result:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Running | Stage::Cancelling) {
            return;
        }
        let request = match self.build(cx) {
            Ok(request) => request,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.stage = Stage::Running;
        self.error = None;
        self.phase = None;
        self.task_index = 0;
        self.task_total = 1;
        self.current_name = None;
        self.completed = 0;
        self.failed = 0;
        self.skipped = 0;
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.results.clear();
        self.messages
            .update(cx, |messages, cx| messages.clear(window, cx));
        self.message(MessageLevel::Info, "Starting prediction".into(), window, cx);
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
            )
        });
    }

    fn handle_message(
        &mut self,
        message: WorkerMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match message {
            WorkerMessage::Log(log) => self
                .messages
                .update(cx, |messages, cx| messages.push(log, window, cx)),
            WorkerMessage::Event(event) => match event {
                RunEvent::PhaseChanged(phase) => self.phase = Some(phase),
                RunEvent::BatchDiscovered { total, skipped } => {
                    self.task_total = total;
                    self.skipped = skipped;
                }
                RunEvent::InputSkipped { input, error } => {
                    self.message(
                        MessageLevel::Warning,
                        format!("Skipped {}: {error}", input.display()),
                        window,
                        cx,
                    );
                    self.results.push(ResultRow {
                        path: input,
                        prediction: None,
                        error: Some(error),
                    });
                }
                RunEvent::TaskStarted {
                    index,
                    total,
                    input,
                    ..
                } => {
                    self.task_index = index;
                    self.task_total = total;
                    self.current_name = Some(input);
                }
                RunEvent::PredictionCompleted { input, prediction } => {
                    self.results.push(ResultRow {
                        path: input,
                        prediction: Some(prediction),
                        error: None,
                    });
                }
                RunEvent::TaskFinished {
                    input,
                    status,
                    error,
                    ..
                } => {
                    if status == TaskStatus::Success {
                        self.completed += 1;
                    } else if status == TaskStatus::Failure {
                        self.failed += 1;
                    }
                    if let Some(error) = error {
                        self.message(
                            MessageLevel::Error,
                            format!("{}: {error}", input.display()),
                            window,
                            cx,
                        );
                        self.results.push(ResultRow {
                            path: input,
                            prediction: None,
                            error: Some(error),
                        });
                    }
                }
                RunEvent::Warning { message } => {
                    self.message(MessageLevel::Warning, message, window, cx)
                }
                RunEvent::Error { message } => {
                    self.message(MessageLevel::Error, message, window, cx)
                }
                _ => {}
            },
            WorkerMessage::Finished(outcome) => {
                self.messages
                    .update(cx, |messages, cx| messages.finish(window, cx));
                self.stage = Stage::Finished(outcome.status());
                self.elapsed = self.started.map(|started| started.elapsed());
                self.cancellation = None;
                match outcome {
                    RunOutcome::Failed(error) => self.error = Some(format!("{error:#}")),
                    RunOutcome::Batch(batch) => {
                        self.task_total = batch.total;
                        self.completed = batch.succeeded;
                        self.failed = batch.failures.len();
                    }
                    RunOutcome::Completed | RunOutcome::Cancelled => {}
                }
                let (level, label) = match self.stage {
                    Stage::Finished(RunStatus::Success) => {
                        (MessageLevel::Info, "Prediction completed")
                    }
                    Stage::Finished(RunStatus::Failure) => {
                        (MessageLevel::Error, "Prediction failed")
                    }
                    Stage::Finished(RunStatus::Cancelled) => {
                        (MessageLevel::Warning, "Prediction cancelled")
                    }
                    _ => unreachable!(),
                };
                self.message(
                    level,
                    self.error
                        .clone()
                        .map_or_else(|| label.into(), |error| format!("{label}: {error}")),
                    window,
                    cx,
                );
            }
        }
        cx.emit(ActivityChanged);
        cx.notify();
    }

    pub fn activity(&self) -> Option<Activity> {
        match self.stage {
            Stage::Idle => None,
            Stage::Running => Some(Activity::Running(format!(
                "{} · {}/{}",
                execution::phase_label(self.phase),
                self.task_index,
                self.task_total
            ))),
            Stage::Cancelling => Some(Activity::Cancelling),
            Stage::Finished(status) => Some(Activity::Finished(status)),
        }
    }

    pub fn set_page_visible(
        &self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.messages.update(cx, |messages, cx| {
            messages.set_page_visible(visible, window, cx)
        });
    }
}

impl Drop for PredictPage {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
    }
}

impl Render for PredictPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        components::page_layout(
            "predict-panes",
            "Predict",
            self.render_work_area(cx).into_any_element(),
            self.render_inspector(cx).into_any_element(),
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Mode, Workspace};
    use gpui_kit::{TestAppContext, component::Root, px, size, test::TestWindowExt as _};

    #[gpui_kit::test]
    fn background_prediction_is_visible_and_reopenable(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            let page = workspace.read(cx).predict.clone();
            page.update(cx, |page, cx| {
                page.stage = Stage::Running;
                page.phase = Some(RunPhase::Predicting);
                page.task_index = 2;
                page.task_total = 5;
                cx.emit(ActivityChanged);
            });
            workspace.update(cx, |workspace, cx| {
                workspace.change_mode(Mode::Emulate, window, cx)
            });
            window.render_frame(cx);
            let view = window.find("view-predict");
            assert!(view.visible());
            assert_eq!(
                view.label(),
                Some("View predict: Running, Predicting · 2/5")
            );
            window.click("view-predict", cx);
            assert!(matches!(workspace.read(cx).mode, Mode::Predict));
        })
        .unwrap();
    }
}
