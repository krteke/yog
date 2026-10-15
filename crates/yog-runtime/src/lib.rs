mod batch;
mod config;
mod diagnostics;
mod emulate;
mod error;
mod jsonl;
mod output;
mod path;
mod progress;
mod recursive;
mod report;
mod transcode;
mod validate;
mod verify;

pub mod control;
#[cfg(feature = "dev-tools")]
#[doc(hidden)]
pub mod dev_tools;
pub mod event;
pub mod recovery;
pub mod terminal;

use control::RunControl;
use diagnostics::Diagnostics;
use error::RunError;
use event::{RunEvent, RunPhase, TaskResult};
use report::Report;
use std::{path::PathBuf, time::Duration};
use transcode::Transcoder;
use yog_core::ffmpeg::{plan::TranscodeRequest, vmaf::VmafOptions};

pub use config::Config;
pub use emulate::{Candidate, EmulationOptions, parse_quality_points};
pub use validate::Validate;

use crate::{
    recovery::RecoveryOptions,
    report::record::{PredictRecord, TaskRecord, TranscodeRecord},
};

#[derive(Debug)]
pub struct Command {
    pub request: TranscodeRequest,
    pub operation: Operation,
    pub recursive: bool,
}

#[derive(Debug, Clone)]
pub enum Operation {
    Transcode,
    Predict,
    Emulate(EmulationOptions),
}

#[derive(Debug, Clone)]
pub struct Options {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub timeout: Option<Duration>,
    pub verbose: bool,
    pub verify: bool,
    pub vmaf: Option<VmafOptions>,
    pub terminal_output: bool,
    pub report: Option<PathBuf>,
    pub overwrite_report: bool,
    pub recovery: RecoveryOptions,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            timeout: None,
            verbose: false,
            verify: false,
            vmaf: None,
            terminal_output: true,
            report: None,
            overwrite_report: false,
            recovery: RecoveryOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Success,
    Failure,
    Cancelled,
}

#[derive(Debug)]
pub enum RunOutcome {
    Completed,
    Failed(anyhow::Error),
    Cancelled,
    Batch(BatchOutcome),
}

impl RunOutcome {
    pub fn status(&self) -> RunStatus {
        match self {
            Self::Completed => RunStatus::Success,
            Self::Failed(_) => RunStatus::Failure,
            Self::Cancelled => RunStatus::Cancelled,
            Self::Batch(batch) if batch.cancelled => RunStatus::Cancelled,
            Self::Batch(batch)
                if !batch.failures.is_empty()
                    || batch.state_error.is_some()
                    || batch.recovery_error.is_some()
                    || (batch.total == batch.skipped
                        && batch.resume_skipped == 0
                        && batch.manual_skipped == 0) =>
            {
                RunStatus::Failure
            }
            Self::Batch(_) => RunStatus::Success,
        }
    }
}

#[derive(Debug, Default)]
pub struct BatchOutcome {
    /// Number of scanned input files, including files subsequently skipped.
    pub total: usize,
    pub succeeded: usize,
    pub skipped: usize,
    /// Subset of skipped tasks explicitly bypassed by a recovery mismatch policy.
    pub resume_skipped: usize,
    /// Subset of skipped tasks manually bypassed, including retained manual skips.
    pub manual_skipped: usize,
    pub reused: usize,
    pub forced: usize,
    pub state_error: Option<anyhow::Error>,
    /// Recovery could not proceed; no task execution or state rewrite took place.
    pub recovery_error: Option<anyhow::Error>,
    pub failures: Vec<TaskFailure>,
    pub cancelled: bool,
}

#[derive(Debug)]
pub struct TaskFailure {
    pub input: PathBuf,
    pub phase: RunPhase,
    pub error: anyhow::Error,
}

/// Executes a command as provided by the caller.
///
/// Validation is intentionally not implicit. Frontends that accept untrusted or
/// loosely structured input can call [`Validate::validate`] on the whole
/// [`Command`] or on individual values before execution. Strongly typed
/// frontends may execute directly.
pub async fn run(
    command: Command,
    options: Options,
    config: Config,
    control: RunControl,
) -> RunOutcome {
    run_with_events(command, options, config, control, |_| {}).await
}

/// Executes a command and forwards structured lifecycle and progress events to
/// the caller. Event delivery is synchronous; handlers must return promptly.
/// Recovery confirmation requests must be retained and answered by the caller.
/// The runtime awaits their reply asynchronously and remains cancellable.
pub async fn run_with_events(
    command: Command,
    options: Options,
    config: Config,
    control: RunControl,
    on_event: impl Fn(RunEvent) + Send + Sync,
) -> RunOutcome {
    let diagnostics = Diagnostics::new(options.verbose, options.terminal_output, &on_event);
    let cancellation = control.cancellation();
    let mut transcoder = Transcoder::new(&options, config, cancellation.clone());
    if command.recursive {
        batch::run(command, &mut transcoder, &diagnostics, &control, &options).await
    } else {
        let mut report = match Report::open(options.report.as_deref(), options.overwrite_report) {
            Ok(report) => report,
            Err(error) => {
                diagnostics.error(&error);
                return RunOutcome::Failed(error);
            }
        };

        let outcome = run_single(command, &transcoder, &diagnostics, &mut report).await;
        report.finish(&diagnostics);
        outcome
    }
}

pub(crate) enum TaskOutcome {
    Success,
    Error(RunError),
}

impl TaskOutcome {
    fn result(&self) -> TaskResult {
        match self {
            Self::Success => TaskResult::Success,
            Self::Error(error) => error.result(),
        }
    }
}

impl From<Result<(), RunError>> for TaskOutcome {
    fn from(value: Result<(), RunError>) -> Self {
        match value {
            Ok(()) => Self::Success,
            Err(error) => Self::Error(error),
        }
    }
}

pub(crate) fn record_task<R: TaskRecord>(
    record: R,
    outcome: &TaskOutcome,
    report: &mut Report,
    diagnostics: &Diagnostics<'_>,
) {
    report.write(&record.finish(outcome.result()), diagnostics);
}

fn finish(outcome: TaskOutcome, diagnostics: &Diagnostics<'_>) -> RunOutcome {
    match outcome {
        TaskOutcome::Success => RunOutcome::Completed,
        TaskOutcome::Error(RunError::Cancelled) => {
            diagnostics.cancelled();
            RunOutcome::Cancelled
        }
        TaskOutcome::Error(error @ RunError::Failed { .. }) => {
            diagnostics.task_error(None, &error);
            let RunError::Failed { error, .. } = error else {
                unreachable!()
            };
            RunOutcome::Failed(error)
        }
        TaskOutcome::Error(RunError::Skipped { .. }) => {
            unreachable!("single tasks are never skipped")
        }
    }
}

async fn run_single(
    command: Command,
    transcoder: &Transcoder,
    diagnostics: &Diagnostics<'_>,
    report: &mut Report,
) -> RunOutcome {
    let Command {
        request, operation, ..
    } = command;

    let input = request.input.clone();
    let output = (!request.output.as_os_str().is_empty()).then(|| request.output.clone());
    diagnostics.event(RunEvent::TaskStarted {
        index: 1,
        total: 1,
        input: input.clone(),
        output,
    });

    let outcome = match operation {
        Operation::Transcode => {
            let mut record = TranscodeRecord::new(&request);
            let result = transcoder.run(request, diagnostics, &mut record).await;
            let outcome = result.into();
            record_task(record, &outcome, report, diagnostics);
            outcome
        }
        Operation::Predict => {
            let mut record = PredictRecord::new(&request, transcoder.prediction_options());
            let result = transcoder.predict(request, diagnostics, &mut record).await;
            let outcome = result.into();
            record_task(record, &outcome, report, diagnostics);
            outcome
        }
        Operation::Emulate(options) => {
            let result = transcoder
                .emulate(request, &options, diagnostics, report)
                .await;
            result.into()
        }
    };
    diagnostics.event(RunEvent::TaskFinished {
        recovery: None,
        index: 1,
        total: 1,
        input,
        result: outcome.result(),
    });
    if matches!(outcome, TaskOutcome::Success) && transcoder.cancelled() {
        diagnostics.cancelled();
        return RunOutcome::Cancelled;
    }
    finish(outcome, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn run_does_not_apply_optional_validation() {
        let cancellation = RunControl::new();
        cancellation.cancel();
        let outcome = run(
            Command {
                request: TranscodeRequest::new("", ""),
                operation: Operation::Transcode,
                recursive: false,
            },
            Options {
                terminal_output: false,
                ..Options::default()
            },
            Config::default(),
            cancellation,
        )
        .await;

        assert!(matches!(outcome, RunOutcome::Cancelled));
    }

    #[tokio::test]
    async fn run_with_events_reports_single_task_lifecycle() {
        let cancellation = RunControl::new();
        cancellation.cancel();
        let received = Arc::new(Mutex::new(Vec::new()));
        let event_log = Arc::clone(&received);

        let outcome = run_with_events(
            Command {
                request: TranscodeRequest::new("input.mkv", "output.mkv"),
                operation: Operation::Transcode,
                recursive: false,
            },
            Options {
                terminal_output: false,
                ..Options::default()
            },
            Config::default(),
            cancellation,
            move |event| {
                let name = match event {
                    RunEvent::TaskStarted {
                        index: 1, total: 1, ..
                    } => "started",
                    RunEvent::TaskFinished {
                        index: 1,
                        total: 1,
                        result: TaskResult::Cancelled,
                        ..
                    } => "cancelled",
                    _ => return,
                };
                event_log.lock().unwrap().push(name);
            },
        )
        .await;

        assert!(matches!(outcome, RunOutcome::Cancelled));
        assert_eq!(*received.lock().unwrap(), ["started", "cancelled"]);
    }
}
