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
#[cfg(test)]
use gpui_kit::test::TestSupportExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, Hsla, InteractiveElement as _, IntoElement,
    ListAlignment, ListState, ParentElement as _, Render, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use tokio_util::sync::CancellationToken;
use yog_core::ffmpeg::{
    encoding::RateControl,
    plan::{TranscodeRequest, VideoAction},
    prediction::Prediction,
};
use yog_runtime::{
    Command, Operation, RunOutcome, RunStatus, Validate,
    event::{RunEvent, RunPhase, TaskResult, TaskStatus},
};

use super::{
    components,
    execution::{self, Activity, ActivityChanged, RunRequest, WorkerMessage},
    messages::{ExpansionChanged, MessageLevel, MessageSource, Messages, NewMessage},
    result_filter::ResultFilter,
    settings::SettingsPanel,
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
    result: TaskResult,
}

pub struct PredictPage {
    source: Entity<SourcePicker>,
    preferences: Entity<SettingsPanel>,
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
    pending_prediction: Option<Prediction>,
    work_list: ListState,
    result_filter: Entity<ResultFilter>,
    _filter_observation: Subscription,
    _rate_subscription: Subscription,
    _video_observation: Subscription,
    _messages_subscription: Subscription,
}

impl EventEmitter<ActivityChanged> for PredictPage {}

impl PredictPage {
    pub fn new(
        source: Entity<SourcePicker>,
        preferences: Entity<SettingsPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let work_list = ListState::new(1, ListAlignment::Top, px(300.));
        let result_filter =
            cx.new(|cx| ResultFilter::new(true, false, work_list.clone(), window, cx));
        let filter_observation = cx.observe(&result_filter, |_, _, cx| cx.notify());
        Self {
            source,
            preferences,
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
            pending_prediction: None,
            work_list,
            result_filter,
            _filter_observation: filter_observation,
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
        let (mut options, config) = self.preferences.read(cx).run_defaults()?;
        options.report = report;
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
        self.pending_prediction = None;
        self.result_filter
            .update(cx, |filter, cx| filter.reset(window, cx));
        self.messages
            .update(cx, |messages, cx| messages.clear(window, cx));
        self.message(MessageLevel::Info, "Starting prediction".into(), window, cx);
        let cancellation = CancellationToken::new();
        self.cancellation = Some(cancellation.clone());
        let mut receiver = execution::spawn(request, cancellation);
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
            WorkerMessage::Log(log) => {
                self.messages
                    .update(cx, |messages, cx| messages.push(log, window, cx));
                return;
            }
            WorkerMessage::Event(event) => match event {
                RunEvent::PhaseChanged(phase) => self.phase = Some(phase),
                RunEvent::BatchDiscovered { total } => {
                    self.task_total = total;
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
                RunEvent::PredictionCompleted { prediction, .. } => {
                    self.pending_prediction = Some(prediction);
                }
                RunEvent::TaskFinished { input, result, .. } => {
                    match result.status() {
                        TaskStatus::Success => self.completed += 1,
                        TaskStatus::Failure => self.failed += 1,
                        TaskStatus::Skipped => self.skipped += 1,
                        TaskStatus::Cancelled => {}
                    }
                    if let Some(message) = result.message() {
                        let level = if result.status() == TaskStatus::Failure {
                            MessageLevel::Error
                        } else {
                            MessageLevel::Warning
                        };
                        self.message(
                            level,
                            format!("{result}: {}: {message}", input.display()),
                            window,
                            cx,
                        );
                    }
                    let prediction = if result.status() == TaskStatus::Success {
                        self.pending_prediction.take()
                    } else {
                        None
                    };
                    self.result_filter
                        .update(cx, |filter, cx| filter.append(&result, window, cx));
                    self.results.push(ResultRow {
                        path: input,
                        prediction,
                        result,
                    });
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
                        self.skipped = batch.skipped;
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
        self.work_list.remeasure_items(0..1);
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

    pub fn set_page_visible(&self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
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
    fn prediction_payload_and_completion_create_one_result_row(cx: &mut TestAppContext) {
        use yog_core::ffmpeg::prediction::{Estimate, QualityPrediction};
        cx.update(gpui_kit::init);
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            let page = workspace.read(cx).predict.clone();
            page.update(cx, |page, cx| {
                let estimate = |value| Estimate {
                    value,
                    low: value,
                    high: value,
                };
                page.handle_message(
                    WorkerMessage::Event(RunEvent::PredictionCompleted {
                        input: "video.mkv".into(),
                        prediction: Prediction {
                            source_bytes: 200,
                            source_duration_seconds: 2.0,
                            sampled_seconds: 1.0,
                            speed: estimate(2.0),
                            transcode_seconds: estimate(1.0),
                            output_bytes: Estimate {
                                value: 100,
                                low: 90,
                                high: 110,
                            },
                            quality: QualityPrediction {
                                frames: 24,
                                vmaf: estimate(95.0),
                                ssim: None,
                                psnr_y_db: None,
                                source_stream_index: 0,
                            },
                            samples: Vec::new(),
                        },
                    }),
                    window,
                    cx,
                );
                assert!(page.results.is_empty());
                page.handle_message(
                    WorkerMessage::Event(RunEvent::TaskFinished {
                        index: 1,
                        total: 1,
                        input: "video.mkv".into(),
                        result: TaskResult::Success,
                    }),
                    window,
                    cx,
                );
                assert_eq!(page.results.len(), 1);
                assert_eq!(page.completed, 1);
                assert_eq!(
                    page.results[0]
                        .prediction
                        .as_ref()
                        .unwrap()
                        .quality
                        .vmaf
                        .value,
                    95.0
                );
                assert!(page.pending_prediction.is_none());
                assert_eq!(page.work_list.item_count(), 2);
            });
            Root::new(workspace, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    #[gpui_kit::test]
    fn cancelled_prediction_is_retained_without_incrementing_failed(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            let page = workspace.read(cx).predict.clone();
            page.update(cx, |page, cx| {
                page.handle_message(
                    WorkerMessage::Event(RunEvent::TaskFinished {
                        index: 1,
                        total: 5,
                        input: "cancelled.mkv".into(),
                        result: TaskResult::Cancelled,
                    }),
                    window,
                    cx,
                );
                assert_eq!(page.results.len(), 1);
                assert_eq!(page.results[0].result.status(), TaskStatus::Cancelled);
                assert_eq!(page.failed, 0);
                assert_eq!(page.completed, 0);
            });
            Root::new(workspace, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    #[gpui_kit::test]
    fn large_prediction_results_only_materialize_near_the_viewport(cx: &mut TestAppContext) {
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
                for index in 0..1_000 {
                    page.handle_message(
                        WorkerMessage::Event(RunEvent::TaskFinished {
                            input: format!("input-{index}.mkv").into(),
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
                }
            });
            assert_eq!(page.read(cx).skipped, 1_000);
            workspace.update(cx, |workspace, cx| {
                workspace.change_mode(Mode::Predict, window, cx)
            });
            page.read(cx).work_list.scroll_to(gpui_kit::ListOffset {
                item_ix: 1,
                offset_in_item: px(0.),
            });
            window.render_frame(cx);
            assert!(window.try_find("predict-result-input-0.mkv").is_some());
            let viewport = window.find("predict-results").bounds();
            let result = window.find("predict-result-input-0.mkv").bounds();
            assert!(
                result.size.width >= viewport.size.width - px(64.),
                "prediction card did not fill the work area: card {result:?}, viewport {viewport:?}"
            );
            assert!(
                window.try_find("predict-result-input-999.mkv").is_none(),
                "offscreen prediction was materialized"
            );
            assert_eq!(page.read(cx).results.len(), 1_000);
            page.read(cx).work_list.scroll_to(gpui_kit::ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
            window.render_frame(cx);
            assert!(window.try_find("result-status-filter").is_some());
            assert!(window.try_find("result-reason-filter").is_some());
            window.scroll(
                "predict-results",
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-1_000_000.))),
                cx,
            );
            window.render_frame(cx);
            assert!(window.find("predict-result-input-999.mkv").visible());
        })
        .unwrap();
    }

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
