use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant},
};

use yog_core::ffmpeg::{prediction::Prediction, progress::Progress};
use yog_runtime::{
    Operation, RunOutcome, RunStatus,
    event::{RunEvent, RunPhase, TaskStatus, VmafOutcome},
};

const MAX_NOTICES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Transcode,
    Predict,
    Emulate,
}

impl RunKind {
    pub fn from_operation(operation: &Operation) -> Self {
        match operation {
            Operation::Transcode => Self::Transcode,
            Operation::Predict => Self::Predict,
            Operation::Emulate(_) => Self::Emulate,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Transcode => "Transcode",
            Self::Predict => "Predict",
            Self::Emulate => "Emulate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStage {
    Running,
    Cancelling,
    Finished(RunStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Warning,
    Error,
}

pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
}

pub struct PredictionSummary {
    pub vmaf: f64,
    pub ssim: Option<f64>,
    pub psnr_y_db: Option<f64>,
    pub output_bytes: u64,
    pub transcode_seconds: f64,
    pub speed: f64,
    pub samples: usize,
}

pub struct EmulationState {
    pub index: usize,
    pub total: usize,
    pub candidate: Option<usize>,
    pub label: String,
    pub parameter: &'static str,
    pub quality: u8,
    pub completed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub vmaf: Option<f64>,
    pub output_bytes: Option<u64>,
}

impl From<&Prediction> for PredictionSummary {
    fn from(prediction: &Prediction) -> Self {
        Self {
            vmaf: prediction.quality.vmaf.value,
            ssim: prediction.quality.ssim.map(|estimate| estimate.value),
            psnr_y_db: prediction.quality.psnr_y_db.map(|estimate| estimate.value),
            output_bytes: prediction.output_bytes.value,
            transcode_seconds: prediction.transcode_seconds.value,
            speed: prediction.speed.value,
            samples: prediction.samples.len(),
        }
    }
}

pub struct RunState {
    pub kind: RunKind,
    pub chart_png: Option<PathBuf>,
    pub chart_svg: Option<PathBuf>,
    pub stage: RunStage,
    pub phase: Option<RunPhase>,
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub task_index: usize,
    pub task_total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration: Option<Duration>,
    pub progress: Progress,
    pub prediction: Option<PredictionSummary>,
    pub emulation: Option<EmulationState>,
    pub notices: VecDeque<Notice>,
    pub spinner: usize,
    started: Instant,
    finished_elapsed: Option<Duration>,
}

impl RunState {
    pub fn new(operation: &Operation) -> Self {
        let (chart_png, chart_svg) = match operation {
            Operation::Emulate(options) => (options.png.clone(), options.svg.clone()),
            Operation::Transcode | Operation::Predict => (None, None),
        };
        Self {
            kind: RunKind::from_operation(operation),
            chart_png,
            chart_svg,
            stage: RunStage::Running,
            phase: None,
            input: None,
            output: None,
            task_index: 0,
            task_total: 1,
            succeeded: 0,
            failed: 0,
            skipped: 0,
            duration: None,
            progress: Progress::default(),
            prediction: None,
            emulation: None,
            notices: VecDeque::new(),
            spinner: 0,
            started: Instant::now(),
            finished_elapsed: None,
        }
    }

    pub fn handle_event(&mut self, event: RunEvent) {
        match event {
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
                self.push_notice(
                    NoticeKind::Warning,
                    format!("Skipped {}: {error}", input.display()),
                );
            }
            RunEvent::TaskStarted {
                index,
                total,
                input,
                output,
            } => {
                self.task_index = index;
                self.task_total = total;
                self.input = Some(input);
                self.output = output;
                self.duration = None;
                self.progress = Progress::default();
                self.prediction = None;
                self.emulation = None;
            }
            RunEvent::Progress { duration, progress } => {
                self.duration = duration;
                self.progress = progress;
            }
            RunEvent::PredictionCompleted { input, prediction } => {
                let summary = PredictionSummary::from(&prediction);
                let name = input
                    .file_name()
                    .unwrap_or(input.as_os_str())
                    .to_string_lossy();
                self.push_notice(
                    NoticeKind::Info,
                    format!(
                        "{name} · VMAF {:.2} · {}",
                        summary.vmaf,
                        format_bytes(summary.output_bytes)
                    ),
                );
                self.prediction = Some(summary);
            }
            RunEvent::EmulationPointStarted {
                index,
                total,
                candidate,
                label,
                parameter,
                quality,
            } => {
                let (completed, succeeded, failed) =
                    self.emulation.as_ref().map_or((0, 0, 0), |state| {
                        (state.completed, state.succeeded, state.failed)
                    });
                self.emulation = Some(EmulationState {
                    index,
                    total,
                    candidate,
                    label,
                    parameter,
                    quality,
                    completed,
                    succeeded,
                    failed,
                    vmaf: None,
                    output_bytes: None,
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
                let state = self
                    .emulation
                    .as_mut()
                    .expect("an emulation point must start before it finishes");
                debug_assert_eq!(state.index, index);
                debug_assert_eq!(state.total, total);
                debug_assert_eq!(state.candidate, candidate);
                debug_assert_eq!(state.quality, quality);
                match status {
                    TaskStatus::Success => {
                        state.completed = index;
                        state.succeeded += 1;
                        state.vmaf = vmaf;
                        state.output_bytes = output_bytes;
                    }
                    TaskStatus::Failure => {
                        state.completed = index;
                        state.failed += 1;
                        if let Some(error) = error {
                            self.push_notice(NoticeKind::Error, error);
                        }
                    }
                    TaskStatus::Cancelled => {}
                }
            }
            RunEvent::VmafFinished {
                options, outcome, ..
            } => match outcome {
                VmafOutcome::Scored(score) => {
                    let mode = options
                        .n_subsample
                        .map_or_else(|| "full".to_owned(), |value| format!("1/{}", value.get()));
                    self.push_notice(NoticeKind::Info, format!("VMAF {score:.3} · {mode}"));
                }
                VmafOutcome::Failed(error) => {
                    self.push_notice(NoticeKind::Warning, format!("VMAF failed: {error}"));
                }
                VmafOutcome::Cancelled => {
                    self.push_notice(NoticeKind::Warning, "VMAF cancelled".to_owned());
                }
            },
            RunEvent::Warning { message } => self.push_notice(NoticeKind::Warning, message),
            RunEvent::Error { message } => self.push_notice(NoticeKind::Error, message),
            RunEvent::TaskFinished { status, error, .. } => match status {
                TaskStatus::Success => {
                    self.succeeded += 1;
                    self.progress.finished = true;
                }
                TaskStatus::Failure => {
                    self.failed += 1;
                    self.progress.finished = true;
                    if let Some(error) = error {
                        self.push_notice(NoticeKind::Error, error);
                    }
                }
                TaskStatus::Cancelled => {}
            },
        }
    }

    pub fn request_cancel(&mut self) {
        self.stage = RunStage::Cancelling;
    }

    pub fn finish(&mut self, outcome: RunOutcome) {
        let status = outcome.status();
        match outcome {
            RunOutcome::Failed(error) => {
                self.push_notice(NoticeKind::Error, format!("{error:#}"));
            }
            RunOutcome::Batch(batch) => {
                self.task_total = batch.total;
                self.succeeded = batch.succeeded;
                self.failed = batch.failures.len();
            }
            RunOutcome::Completed | RunOutcome::Cancelled => {}
        }
        self.finished_elapsed = Some(self.started.elapsed());
        self.stage = RunStage::Finished(status);
    }

    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
    }

    pub fn elapsed(&self) -> Duration {
        self.finished_elapsed
            .unwrap_or_else(|| self.started.elapsed())
    }

    pub fn progress_ratio(&self) -> f64 {
        if self.stage == RunStage::Finished(RunStatus::Success) {
            return 1.0;
        }
        if self.kind == RunKind::Emulate {
            return self
                .emulation
                .as_ref()
                .map_or(0.0, |state| state.completed as f64 / state.total as f64);
        }
        if self.task_index == 0 || self.task_total == 0 {
            return 0.0;
        }

        let task = match (
            self.progress.finished,
            self.duration,
            self.progress.out_time_us,
        ) {
            (true, _, _) => 1.0,
            (false, Some(duration), Some(position)) if !duration.is_zero() => {
                let position = position.max(0) as f64 / 1_000_000.0;
                (position / duration.as_secs_f64()).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        ((self.task_index - 1) as f64 + task) / self.task_total as f64
    }

    fn push_notice(&mut self, kind: NoticeKind, text: String) {
        if self
            .notices
            .back()
            .is_some_and(|notice| notice.text == text)
        {
            return;
        }
        if self.notices.len() == MAX_NOTICES {
            self.notices.pop_front();
        }
        self.notices.push_back(Notice { kind, text });
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1_048_576.0;
    const GIB: f64 = 1_073_741_824.0;
    if bytes as f64 >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB)
    } else {
        format!("{:.0} MiB", bytes as f64 / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use yog_core::ffmpeg::prediction::{Estimate, QualityPrediction};

    #[test]
    fn task_progress_combines_completed_tasks_with_the_current_task() {
        let mut state = RunState::new(&Operation::Transcode);
        state.handle_event(RunEvent::TaskStarted {
            index: 2,
            total: 4,
            input: "input.mkv".into(),
            output: Some("output.mkv".into()),
        });
        state.handle_event(RunEvent::Progress {
            duration: Some(Duration::from_secs(10)),
            progress: Progress {
                out_time_us: Some(5_000_000),
                ..Progress::default()
            },
        });

        assert_eq!(state.progress_ratio(), 0.375);
    }

    #[test]
    fn vmaf_failure_is_a_warning_and_does_not_fail_the_task() {
        let mut state = RunState::new(&Operation::Transcode);
        state.handle_event(RunEvent::VmafFinished {
            output: "output.mkv".into(),
            options: Default::default(),
            outcome: VmafOutcome::Failed("model unavailable".to_owned()),
        });
        state.handle_event(RunEvent::TaskFinished {
            index: 1,
            total: 1,
            input: "input.mkv".into(),
            status: TaskStatus::Success,
            error: None,
        });

        assert_eq!(state.succeeded, 1);
        assert_eq!(state.failed, 0);
        assert_eq!(state.notices.len(), 1);
        assert_eq!(state.notices[0].kind, NoticeKind::Warning);
    }

    #[test]
    fn prediction_event_keeps_the_central_estimates_for_the_result_screen() {
        let mut state = RunState::new(&Operation::Predict);
        state.handle_event(RunEvent::PredictionCompleted {
            input: "movie.mkv".into(),
            prediction: Prediction {
                source_bytes: 1_000,
                source_duration_seconds: 60.0,
                sampled_seconds: 10.0,
                speed: estimate(2.5),
                transcode_seconds: estimate(24.0),
                output_bytes: Estimate {
                    value: 838_860_800,
                    low: 800_000_000,
                    high: 900_000_000,
                },
                quality: QualityPrediction {
                    frames: 240,
                    vmaf: estimate(95.25),
                    ssim: Some(estimate(0.9987)),
                    psnr_y_db: Some(estimate(42.5)),
                    source_stream_index: 0,
                },
                samples: Vec::new(),
            },
        });

        let prediction = state.prediction.as_ref().unwrap();
        assert_eq!(prediction.vmaf, 95.25);
        assert_eq!(prediction.output_bytes, 838_860_800);
        assert_eq!(prediction.transcode_seconds, 24.0);
        assert_eq!(prediction.speed, 2.5);
        assert_eq!(prediction.ssim, Some(0.9987));
        assert_eq!(prediction.psnr_y_db, Some(42.5));
        assert_eq!(state.notices[0].text, "movie.mkv · VMAF 95.25 · 800 MiB");
    }

    #[test]
    fn emulation_progress_counts_completed_successes_and_failures() {
        let mut state = RunState::new(&Operation::Emulate(yog_runtime::EmulationOptions {
            png: Some("chart.png".into()),
            svg: None,
            qualities: vec![20, 21],
            candidates: Vec::new(),
        }));
        state.handle_event(RunEvent::EmulationPointStarted {
            index: 1,
            total: 2,
            candidate: None,
            label: "x265 / Software / Preset medium".to_owned(),
            parameter: "CRF",
            quality: 20,
        });
        state.handle_event(RunEvent::EmulationPointFinished {
            index: 1,
            total: 2,
            candidate: None,
            quality: 20,
            status: TaskStatus::Success,
            vmaf: Some(95.5),
            output_bytes: Some(838_860_800),
            error: None,
        });

        assert_eq!(state.progress_ratio(), 0.5);
        assert_eq!(state.emulation.as_ref().unwrap().succeeded, 1);
        assert_eq!(state.emulation.as_ref().unwrap().vmaf, Some(95.5));

        state.handle_event(RunEvent::EmulationPointStarted {
            index: 2,
            total: 2,
            candidate: None,
            label: "x265 / Software / Preset medium".to_owned(),
            parameter: "CRF",
            quality: 21,
        });
        state.handle_event(RunEvent::EmulationPointFinished {
            index: 2,
            total: 2,
            candidate: None,
            quality: 21,
            status: TaskStatus::Failure,
            vmaf: None,
            output_bytes: None,
            error: Some("prediction failed".to_owned()),
        });

        assert_eq!(state.progress_ratio(), 1.0);
        assert_eq!(state.emulation.as_ref().unwrap().succeeded, 1);
        assert_eq!(state.emulation.as_ref().unwrap().failed, 1);
        assert_eq!(state.notices[0].text, "prediction failed");
    }

    fn estimate<T: Copy>(value: T) -> Estimate<T> {
        Estimate {
            value,
            low: value,
            high: value,
        }
    }
}
