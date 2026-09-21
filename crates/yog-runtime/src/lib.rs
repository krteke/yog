mod config;
mod diagnostics;
mod emulate;
mod error;
mod output;
mod progress;
mod recursive;
mod report;
mod transcode;
mod validate;
mod verify;

use diagnostics::Diagnostics;
use error::RunError;
use report::{Record, Report, Status};
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;
use transcode::Transcoder;
use yog_core::ffmpeg::{plan::TranscodeRequest, vmaf::VmafOptions};

pub use config::Config;
pub use emulate::{Candidate, EmulationOptions};
pub use validate::Validate;

use crate::report::record::{PredictRecord, SkippedRecord, TaskRecord, TranscodeRecord};

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
            Self::Batch(batch) if !batch.failures.is_empty() => RunStatus::Failure,
            Self::Batch(_) => RunStatus::Success,
        }
    }
}

#[derive(Debug)]
pub struct BatchOutcome {
    pub total: usize,
    pub succeeded: usize,
    pub failures: Vec<TaskFailure>,
    pub cancelled: bool,
}

#[derive(Debug)]
pub struct TaskFailure {
    pub input: PathBuf,
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
    cancellation: CancellationToken,
) -> RunOutcome {
    let diagnostics = Diagnostics::new(options.verbose, options.terminal_output);
    let mut report = match Report::try_from(options.report.as_deref()) {
        Ok(report) => report,
        Err(error) => {
            diagnostics.error(&error);
            return RunOutcome::Failed(error);
        }
    };

    let transcoder = Transcoder::new(&options, config, cancellation.clone());
    let outcome = if command.recursive {
        run_batch(
            command,
            &transcoder,
            &diagnostics,
            &cancellation,
            &mut report,
        )
        .await
    } else {
        run_single(command, &transcoder, &diagnostics, &mut report).await
    };
    report.finish(&diagnostics);

    outcome
}

pub(crate) enum TaskOutcome {
    Success,
    Failed(anyhow::Error),
    Cancelled,
}

impl From<Result<(), RunError>> for TaskOutcome {
    fn from(value: Result<(), RunError>) -> Self {
        match value {
            Ok(()) => Self::Success,
            Err(RunError::Cancelled) => Self::Cancelled,
            Err(RunError::Failed(error)) => Self::Failed(error),
        }
    }
}

impl From<anyhow::Error> for TaskOutcome {
    fn from(error: anyhow::Error) -> Self {
        Err(RunError::from(error)).into()
    }
}

fn finish_record<R: TaskRecord>(record: R, outcome: &TaskOutcome) -> Record {
    match outcome {
        TaskOutcome::Success => record.finish(Status::Success, None),
        TaskOutcome::Cancelled => record.finish(Status::Cancelled, None),
        TaskOutcome::Failed(error) => record.finish(Status::Failure, Some(format!("{error:#}"))),
    }
}

pub(crate) fn record_task<R: TaskRecord>(
    record: R,
    outcome: &TaskOutcome,
    report: &mut Report,
    diagnostics: &Diagnostics,
) {
    report.write(&finish_record(record, outcome), diagnostics);
}

fn finish(outcome: TaskOutcome, diagnostics: &Diagnostics) -> RunOutcome {
    match outcome {
        TaskOutcome::Success => RunOutcome::Completed,
        TaskOutcome::Cancelled => {
            diagnostics.cancelled();
            RunOutcome::Cancelled
        }
        TaskOutcome::Failed(error) => {
            diagnostics.error(&error);
            RunOutcome::Failed(error)
        }
    }
}

async fn run_single(
    command: Command,
    transcoder: &Transcoder,
    diagnostics: &Diagnostics,
    report: &mut Report,
) -> RunOutcome {
    let Command {
        request, operation, ..
    } = command;

    match operation {
        Operation::Transcode => {
            let mut record = TranscodeRecord::new(&request);
            let result = transcoder.run(request, diagnostics, &mut record).await;
            let outcome = result.into();
            record_task(record, &outcome, report, diagnostics);
            finish(outcome, diagnostics)
        }
        Operation::Predict => {
            let mut record = PredictRecord::new(&request, transcoder.prediction_options());
            let result = transcoder.predict(request, diagnostics, &mut record).await;
            let outcome = result.into();
            record_task(record, &outcome, report, diagnostics);
            finish(outcome, diagnostics)
        }
        Operation::Emulate(options) => {
            let result = transcoder
                .emulate(request, &options, diagnostics, report)
                .await;
            finish(result.into(), diagnostics)
        }
    }
}

async fn run_batch(
    command: Command,
    transcoder: &Transcoder,
    diagnostics: &Diagnostics,
    cancellation: &CancellationToken,
    report: &mut Report,
) -> RunOutcome {
    let Command {
        request, operation, ..
    } = command;
    let predict = matches!(operation, Operation::Predict);
    let input_root = request.input.clone();
    let discovery = match recursive::discover(request, transcoder, diagnostics, predict).await {
        Ok(discovery) => discovery,
        Err(RunError::Cancelled) => {
            diagnostics.cancelled();
            return RunOutcome::Cancelled;
        }
        Err(RunError::Failed(error)) => {
            diagnostics.error(&error);
            return RunOutcome::Failed(error);
        }
    };
    let total = discovery.tasks.len();
    let mut succeeded = 0;
    let mut failures = Vec::new();

    for skipped in &discovery.skipped {
        report.write(
            &Record::Skipped(SkippedRecord::new(&skipped.input, &skipped.error)),
            diagnostics,
        );
    }

    if discovery.tasks.is_empty() {
        let error = anyhow::anyhow!("no video files found in {}", input_root.display());
        diagnostics.error(&error);
        return RunOutcome::Failed(error);
    }

    for (request, media) in discovery.tasks {
        let input = request.input.clone();
        diagnostics.begin_task();
        let (line, outcome) = match &operation {
            Operation::Predict => {
                let mut record = PredictRecord::new(&request, transcoder.prediction_options());
                let result = transcoder
                    .predict_probed(request, media, diagnostics, &mut record)
                    .await;
                let outcome = result.into();
                let line = finish_record(record, &outcome);
                (line, outcome)
            }
            Operation::Transcode => {
                let mut record = TranscodeRecord::new(&request);
                let result = transcoder
                    .run_probed(request, media, diagnostics, &mut record)
                    .await;
                let outcome = result.into();
                let line = finish_record(record, &outcome);
                (line, outcome)
            }
            Operation::Emulate(_) => unreachable!("recursive emulation is unsupported"),
        };
        report.write(&line, diagnostics);
        match outcome {
            TaskOutcome::Success => succeeded += 1,
            TaskOutcome::Failed(error) => {
                diagnostics.task_error(&input, &error);
                failures.push(TaskFailure { input, error });
            }
            TaskOutcome::Cancelled => break,
        }
    }

    let cancelled = cancellation.is_cancelled();
    diagnostics.batch_summary(total, succeeded, failures.len(), cancelled);
    RunOutcome::Batch(BatchOutcome {
        total,
        succeeded,
        failures,
        cancelled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_does_not_apply_optional_validation() {
        let cancellation = CancellationToken::new();
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
}
