use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant},
};

use yog_core::ffmpeg::progress::Progress;
use yog_runtime::{
    RunOutcome, RunStatus,
    event::{RunEvent, RunPhase, TaskStatus, VmafOutcome},
};

const MAX_NOTICES: usize = 100;

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

pub struct RunState {
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
    pub notices: VecDeque<Notice>,
    pub spinner: usize,
    started: Instant,
    finished_elapsed: Option<Duration>,
}

impl RunState {
    pub fn new() -> Self {
        Self {
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
            }
            RunEvent::Progress { duration, progress } => {
                self.duration = duration;
                self.progress = progress;
            }
            RunEvent::PredictionCompleted { .. }
            | RunEvent::EmulationPointStarted { .. }
            | RunEvent::EmulationPointFinished { .. } => {}
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

    #[test]
    fn task_progress_combines_completed_tasks_with_the_current_task() {
        let mut state = RunState::new();
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
        let mut state = RunState::new();
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
}
