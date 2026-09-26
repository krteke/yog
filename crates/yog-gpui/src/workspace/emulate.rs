mod candidate;
mod view;

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    form::{Field, Form},
    input::{Input, InputState},
    progress::Progress as ProgressBar,
    scroll::ScrollableElement,
    select::{SearchableVec, Select, SelectState},
    tag::Tag,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, ImageSource, IntoElement, ObjectFit,
    ParentElement as _, Render, Styled as _, StyledImage as _, Subscription, Window, div, img,
    prelude::FluentBuilder as _,
};
#[cfg(test)]
use gpui_kit::{InteractiveElement as _, test::TestSupportExt as _};
use tokio_util::sync::CancellationToken;
use yog_core::ffmpeg::{
    decoding::DecodingBackend,
    plan::{TranscodeRequest, VideoAction},
};
use yog_runtime::{
    Candidate, Command, Config, EmulationOptions, Operation, RunOutcome, RunStatus, Validate,
    event::{RunEvent, RunPhase, TaskStatus},
    parse_quality_points,
};

use super::{
    components,
    execution::{self, Activity, ActivityChanged, RunRequest, WorkerMessage},
    messages::{ExpansionChanged, MessageLevel, MessageSource, Messages, NewMessage},
    settings::SettingsPanel,
    source::SourcePicker,
    video::{CONTAINERS, Choice, VideoInputs, container, selected},
};
use candidate::CandidateEditor;

#[derive(Clone, Copy)]
enum Destination {
    Png,
    Svg,
    Report,
}

enum Stage {
    Idle,
    Running,
    Cancelling,
    Finished(RunStatus),
}

struct Point {
    candidate: Option<usize>,
    label: String,
    parameter: &'static str,
    quality: u8,
    status: TaskStatus,
    vmaf: Option<f64>,
    bytes: Option<u64>,
    error: Option<String>,
}

struct CurrentPoint {
    index: usize,
    candidate: Option<usize>,
    label: String,
    parameter: &'static str,
    quality: u8,
}

pub struct EmulatePage {
    source: Entity<SourcePicker>,
    preferences: Entity<SettingsPanel>,
    video: Entity<VideoInputs>,
    container: Choice,
    quality_points: Entity<InputState>,
    candidates: Vec<Entity<CandidateEditor>>,
    png: Entity<InputState>,
    svg: Entity<InputState>,
    report: Entity<InputState>,
    overwrite: bool,
    messages: Entity<Messages>,
    stage: Stage,
    cancellation: Option<CancellationToken>,
    error: Option<String>,
    destination_error: Option<String>,
    phase: Option<RunPhase>,
    current: Option<CurrentPoint>,
    total: usize,
    succeeded: usize,
    failed: usize,
    points: Vec<Point>,
    chart_paths: Vec<PathBuf>,
    chart_aspect_ratio: f32,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    _video_observation: Subscription,
    _messages_subscription: Subscription,
}

impl EventEmitter<ActivityChanged> for EmulatePage {}

impl EmulatePage {
    pub fn new(
        source: Entity<SourcePicker>,
        preferences: Entity<SettingsPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let video = cx.new(|cx| VideoInputs::new(window, cx));
        let video_observation = cx.observe(&video, |_, _, cx| cx.notify());
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
        let messages = cx.new(|cx| Messages::new(window, cx));
        let messages_subscription =
            cx.subscribe(&messages, |_, _, _: &ExpansionChanged, cx| cx.notify());
        let chart_size = Config::default().emulation;
        Self {
            source,
            preferences,
            video,
            container,
            quality_points: cx.new(|cx| InputState::new(window, cx).default_value("20,24,28,32")),
            candidates: Vec::new(),
            png: cx.new(|cx| InputState::new(window, cx).placeholder("Save PNG chart")),
            svg: cx.new(|cx| InputState::new(window, cx).placeholder("Save SVG chart")),
            report: cx.new(|cx| InputState::new(window, cx).placeholder("Optional report path")),
            overwrite: false,
            messages,
            stage: Stage::Idle,
            cancellation: None,
            error: None,
            destination_error: None,
            phase: None,
            current: None,
            total: 0,
            succeeded: 0,
            failed: 0,
            points: Vec::new(),
            chart_paths: Vec::new(),
            chart_aspect_ratio: chart_size.width.get() as f32 / chart_size.height.get() as f32,
            started: None,
            elapsed: None,
            _video_observation: video_observation,
            _messages_subscription: messages_subscription,
        }
    }

    fn choose_destination(
        &mut self,
        format: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = self
            .source
            .read(cx)
            .path()
            .and_then(|path| path.parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = self
            .source
            .read(cx)
            .path()
            .and_then(|path| path.file_stem())
            .and_then(|stem| stem.to_str())
            .unwrap_or("output");
        let suggested_name = match format {
            Destination::Png => format!("{name}-curve.png"),
            Destination::Svg => format!("{name}-curve.svg"),
            Destination::Report => format!("{name}-emulation.jsonl"),
        };
        let response = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn_in(window, async move |this, cx| match response.await {
            Ok(Ok(Some(path))) => {
                _ = this.update_in(cx, |this, window, cx| {
                    let input = match format {
                        Destination::Png => &this.png,
                        Destination::Svg => &this.svg,
                        Destination::Report => &this.report,
                    };
                    input.update(cx, |input, cx| {
                        input.set_value(path.to_string_lossy().to_string(), window, cx)
                    });
                    this.destination_error = None;
                    cx.notify();
                });
            }
            Ok(Ok(None)) => {}
            result => {
                _ = this.update_in(cx, |this, _, cx| {
                    this.destination_error =
                        Some(format!("Could not open save dialog: {result:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn add_candidate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Running | Stage::Cancelling) {
            return;
        }
        let qualities = self.quality_points.read(cx).value().to_string();
        self.candidates
            .push(cx.new(|cx| CandidateEditor::new(qualities, window, cx)));
        cx.notify();
    }

    fn remove_candidate(&mut self, index: usize, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Running | Stage::Cancelling) {
            return;
        }
        self.candidates.remove(index);
        cx.notify();
    }

    fn build(&self, cx: &mut Context<Self>) -> Result<RunRequest, String> {
        let source = self.source.read(cx);
        let input = source.valid_path().ok_or("Select a video file")?;
        if source.is_directory() {
            return Err("Emulation requires a video file".into());
        }
        let png = self.png.read(cx).value();
        let svg = self.svg.read(cx).value();
        let png = (!png.trim().is_empty()).then(|| PathBuf::from(png.trim()));
        let svg = (!svg.trim().is_empty()).then(|| PathBuf::from(svg.trim()));
        let candidate_mode = !self.candidates.is_empty();
        let (base_decoding, base_encoding) = self.video.read(cx).build(cx);
        let base_qualities =
            parse_quality_points(&self.quality_points.read(cx).value()).map_err(|error| {
                if candidate_mode {
                    format!("Candidate #1: {error}")
                } else {
                    error
                }
            })?;
        let (decoding, video, qualities, candidates) = if candidate_mode {
            let mut candidates = Vec::with_capacity(self.candidates.len() + 1);
            candidates.push(Candidate {
                decoding: base_decoding,
                encoding: base_encoding,
                qualities: base_qualities,
            });
            for (index, editor) in self.candidates.iter().enumerate() {
                candidates.push(
                    editor
                        .read(cx)
                        .build(cx)
                        .map_err(|error| format!("Candidate #{}: {error}", index + 2))?,
                );
            }
            (
                DecodingBackend::Software,
                VideoAction::Copy,
                Vec::new(),
                candidates,
            )
        } else {
            (
                base_decoding,
                VideoAction::Encode(base_encoding),
                base_qualities,
                Vec::new(),
            )
        };
        let mut request = TranscodeRequest::new(input, PathBuf::new())
            .with_decoding(decoding)
            .with_video(video)
            .with_overwrite(self.overwrite);
        if let Some(container) = container(selected(&self.container, cx)) {
            request = request.with_container(container);
        }
        let command = Command {
            request,
            operation: Operation::Emulate(EmulationOptions {
                png,
                svg,
                qualities,
                candidates,
            }),
            recursive: false,
        };
        command.validate().map_err(|error| format!("{error:#}"))?;
        let report = self.report.read(cx).value();
        let report = (!report.trim().is_empty()).then(|| PathBuf::from(report.trim()));
        let Operation::Emulate(chart_outputs) = &command.operation else {
            unreachable!()
        };
        if report.as_ref().is_some_and(|report| {
            execution::same_path(report, input)
                || [chart_outputs.png.as_ref(), chart_outputs.svg.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|chart| execution::same_path(report, chart))
        }) {
            return Err("Report path must differ from the source and charts".into());
        }
        let (mut options, config) = self.preferences.read(cx).run_defaults()?;
        options.report = report;
        Ok(RunRequest {
            command,
            options,
            config,
        })
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
        self.chart_aspect_ratio = request.config.emulation.width.get() as f32
            / request.config.emulation.height.get() as f32;
        self.chart_paths.clear();
        self.stage = Stage::Running;
        self.error = None;
        self.destination_error = None;
        self.phase = None;
        self.current = None;
        self.total = 0;
        self.succeeded = 0;
        self.failed = 0;
        self.points.clear();
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.messages
            .update(cx, |messages, cx| messages.clear(window, cx));
        self.message(MessageLevel::Info, "Starting emulation".into(), window, cx);
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
                RunEvent::EmulationPointStarted {
                    index,
                    total,
                    candidate,
                    label,
                    parameter,
                    quality,
                } => {
                    self.total = total;
                    self.current = Some(CurrentPoint {
                        index,
                        candidate,
                        label,
                        parameter,
                        quality,
                    });
                }
                RunEvent::EmulationPointFinished {
                    index,
                    total,
                    candidate,
                    quality,
                    status,
                    vmaf,
                    output_bytes,
                    error,
                } => {
                    let current = self
                        .current
                        .take()
                        .expect("emulation point must start before it finishes");
                    debug_assert_eq!(
                        (current.index, current.candidate, current.quality),
                        (index, candidate, quality)
                    );
                    self.total = total;
                    match status {
                        TaskStatus::Success => self.succeeded += 1,
                        TaskStatus::Failure => self.failed += 1,
                        TaskStatus::Cancelled => {}
                    }
                    if let Some(error) = &error {
                        self.message(
                            MessageLevel::Error,
                            format!("{} {}: {error}", current.parameter, quality),
                            window,
                            cx,
                        );
                    }
                    self.points.push(Point {
                        candidate,
                        label: current.label,
                        parameter: current.parameter,
                        quality,
                        status,
                        vmaf,
                        bytes: output_bytes,
                        error,
                    });
                }
                RunEvent::EmulationChartRendered { png, svg } => {
                    self.chart_paths = [png, svg].into_iter().flatten().collect();
                    for path in &self.chart_paths {
                        ImageSource::from(path.clone()).remove_asset(cx);
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
                self.elapsed = self.started.map(|start| start.elapsed());
                self.cancellation = None;
                self.current = None;
                if let RunOutcome::Failed(error) = outcome {
                    self.error = Some(format!("{error:#}"));
                }
                let (level, label) = match self.stage {
                    Stage::Finished(RunStatus::Success) => (MessageLevel::Info, "Chart generated"),
                    Stage::Finished(RunStatus::Failure) => {
                        (MessageLevel::Error, "Emulation failed")
                    }
                    Stage::Finished(RunStatus::Cancelled) => {
                        (MessageLevel::Warning, "Emulation cancelled")
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
                self.points.len(),
                self.total
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

impl Drop for EmulatePage {
    fn drop(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
    }
}

impl Render for EmulatePage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        components::page_layout(
            "emulate-panes",
            "Emulate",
            self.render_work_area(cx).into_any_element(),
            self.render_inspector(cx).into_any_element(),
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use gpui_kit::{TestAppContext, component::Root, px, size, test::TestWindowExt as _};

    #[gpui_kit::test]
    fn chart_preview_contains_its_image(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let chart = directory.path().join("chart.svg");
        std::fs::write(
            &chart,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="900" viewBox="0 0 1600 900"><rect width="1600" height="900" fill="white"/></svg>"#,
        )
        .unwrap();

        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            let page = workspace.read(cx).emulate.clone();
            page.update(cx, |page, cx| {
                page.handle_message(
                    WorkerMessage::Event(RunEvent::EmulationChartRendered {
                        png: None,
                        svg: Some(chart),
                    }),
                    window,
                    cx,
                );
            });
            workspace.update(cx, |workspace, cx| {
                workspace.change_mode(super::super::Mode::Emulate, window, cx)
            });
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let card = window.find("emulate-chart-preview").bounds();
            let image = window.find("emulate-chart-image").bounds();
            assert!(image.top() >= card.top(), "image starts above preview");
            assert!(image.bottom() <= card.bottom(), "image exceeds preview");
            assert!(
                ((image.size.width / image.size.height) - 16.0 / 9.0).abs() < 0.01,
                "preview must retain the chart aspect ratio: {image:?}"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn rendered_chart_remains_available_when_some_points_failed(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut workspace = None;
        let handle = cx.open_window(size(px(900.), px(700.)), |window, cx| {
            let view = cx.new(|cx| Workspace::new(window, cx));
            workspace = Some(view.clone());
            Root::new(view, window, cx)
        });
        let workspace = workspace.unwrap();

        cx.update_window(handle.into(), |_, window, cx| {
            let page = workspace.read(cx).emulate.clone();
            page.update(cx, |page, cx| {
                page.handle_message(
                    WorkerMessage::Event(RunEvent::EmulationChartRendered {
                        png: None,
                        svg: Some(PathBuf::from("chart.svg")),
                    }),
                    window,
                    cx,
                );
                page.handle_message(
                    WorkerMessage::Finished(RunOutcome::Failed(
                        std::io::Error::other("one quality point failed").into(),
                    )),
                    window,
                    cx,
                );
            });
            workspace.update(cx, |workspace, cx| {
                workspace.change_mode(super::super::Mode::Emulate, window, cx)
            });
            window.render_frame(cx);
            assert!(window.find("reveal-chart-chart.svg").visible());
            assert!(matches!(
                page.read(cx).stage,
                Stage::Finished(RunStatus::Failure)
            ));
        })
        .unwrap();
    }
}
