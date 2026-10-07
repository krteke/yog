mod config;
#[cfg(feature = "dev-tools")]
#[doc(hidden)]
pub mod dev_tools;
mod diagnostics;
mod emulate;
mod error;
pub mod event;
mod output;
mod progress;
mod recursive;
mod report;
mod transcode;
mod validate;
mod verify;

use diagnostics::Diagnostics;
use error::RunError;
use event::{RunEvent, TaskStatus};
use report::{Record, Report, Status};
use std::{collections::HashSet, path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;
use transcode::Transcoder;
use yog_core::ffmpeg::{plan::TranscodeRequest, vmaf::VmafOptions};

pub use config::Config;
pub use emulate::{Candidate, EmulationOptions, parse_quality_points};
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
            Self::Batch(batch) if !batch.failures.is_empty() || batch.total == batch.skipped => {
                RunStatus::Failure
            }
            Self::Batch(_) => RunStatus::Success,
        }
    }
}

#[derive(Debug)]
pub struct BatchOutcome {
    /// Number of scanned input files, including files subsequently skipped.
    pub total: usize,
    pub succeeded: usize,
    pub skipped: usize,
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
    run_with_events(command, options, config, cancellation, |_| {}).await
}

/// Executes a command and forwards structured lifecycle and progress events to
/// the caller. Event delivery is synchronous; handlers must return promptly.
pub async fn run_with_events(
    command: Command,
    options: Options,
    config: Config,
    cancellation: CancellationToken,
    on_event: impl Fn(RunEvent) + Send + Sync,
) -> RunOutcome {
    let diagnostics = Diagnostics::new(options.verbose, options.terminal_output, &on_event);
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

impl TaskOutcome {
    fn status(&self) -> TaskStatus {
        match self {
            Self::Success => TaskStatus::Success,
            Self::Failed(_) => TaskStatus::Failure,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }

    fn error(&self) -> Option<String> {
        match self {
            Self::Failed(error) => Some(format!("{error:#}")),
            Self::Success | Self::Cancelled => None,
        }
    }
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
    diagnostics: &Diagnostics<'_>,
) {
    report.write(&finish_record(record, outcome), diagnostics);
}

fn finish(outcome: TaskOutcome, diagnostics: &Diagnostics<'_>) -> RunOutcome {
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
        index: 1,
        total: 1,
        input,
        status: outcome.status(),
        error: outcome.error(),
    });
    if matches!(outcome, TaskOutcome::Success) && transcoder.cancelled() {
        diagnostics.cancelled();
        return RunOutcome::Cancelled;
    }
    finish(outcome, diagnostics)
}

async fn run_batch(
    command: Command,
    transcoder: &Transcoder,
    diagnostics: &Diagnostics<'_>,
    cancellation: &CancellationToken,
    report: &mut Report,
) -> RunOutcome {
    let Command {
        request: template,
        operation,
        ..
    } = command;
    let predict = matches!(operation, Operation::Predict);
    diagnostics.event(RunEvent::PhaseChanged(event::RunPhase::Discovering));
    let scan_template = template.clone();
    let scan_cancellation = cancellation.clone();
    let tasks = match tokio::task::spawn_blocking(move || {
        recursive::scan(scan_template, scan_cancellation, predict)
    })
    .await
    .expect("batch path scan panicked")
    {
        Ok(tasks) => tasks,
        Err(RunError::Cancelled) => {
            diagnostics.cancelled();
            return RunOutcome::Cancelled;
        }
        Err(RunError::Failed(error)) => {
            diagnostics.error(&error);
            return RunOutcome::Failed(error);
        }
    };
    let total = tasks.len();
    diagnostics.event(RunEvent::BatchDiscovered { total });
    let mut succeeded = 0;
    let mut failures = Vec::new();
    let mut preflight_skipped = HashSet::new();
    // Without an explicit container, relative paths and suffixes are preserved,
    // so distinct input paths cannot map to the same output path.
    if !predict && template.container.is_some() {
        match recursive::check_output_conflicts(
            &tasks,
            &template,
            transcoder,
            diagnostics,
            report,
            &mut preflight_skipped,
        )
        .await
        {
            Ok(()) | Err(RunError::Cancelled) => {}
            Err(RunError::Failed(error)) => {
                diagnostics.error(&error);
                return RunOutcome::Failed(error);
            }
        }
    }
    let mut skipped = preflight_skipped.len();

    for (offset, task) in tasks.into_iter().enumerate() {
        if cancellation.is_cancelled() {
            break;
        }
        if preflight_skipped.contains(&offset) {
            continue;
        }
        let request = TranscodeRequest {
            input: task.input,
            output: task.output,
            ..template.clone()
        };
        let index = offset + 1;
        let input = request.input.clone();
        let output = (!request.output.as_os_str().is_empty()).then(|| request.output.clone());
        diagnostics.event(RunEvent::TaskStarted {
            index,
            total,
            input: input.clone(),
            output,
        });
        diagnostics.begin_task();
        diagnostics.event(RunEvent::PhaseChanged(event::RunPhase::Probing));
        let outcome = match recursive::probe_task(&request, transcoder, predict).await {
            Err(RunError::Failed(error)) => {
                skipped += 1;
                diagnostics.skipped_input(&input, &error);
                report.write(
                    &Record::Skipped(SkippedRecord::new(&input, &error)),
                    diagnostics,
                );
                continue;
            }
            Err(RunError::Cancelled) => TaskOutcome::Cancelled,
            Ok(media) => match &operation {
                Operation::Predict => {
                    let mut record = PredictRecord::new(&request, transcoder.prediction_options());
                    let result = transcoder
                        .predict_probed(request, media, diagnostics, &mut record)
                        .await;
                    let outcome = result.into();
                    record_task(record, &outcome, report, diagnostics);
                    outcome
                }
                Operation::Transcode => {
                    let mut record = TranscodeRecord::new(&request);
                    let result = transcoder
                        .run_probed(request, media, diagnostics, &mut record)
                        .await;
                    let outcome = result.into();
                    record_task(record, &outcome, report, diagnostics);
                    outcome
                }
                Operation::Emulate(_) => unreachable!("recursive emulation is unsupported"),
            },
        };
        diagnostics.event(RunEvent::TaskFinished {
            index,
            total,
            input: input.clone(),
            status: outcome.status(),
            error: outcome.error(),
        });
        match outcome {
            TaskOutcome::Success => succeeded += 1,
            TaskOutcome::Failed(error) => {
                diagnostics.task_error(&input, &error);
                failures.push(TaskFailure { input, error });
            }
            TaskOutcome::Cancelled => break,
        }
        if cancellation.is_cancelled() {
            break;
        }
    }

    let cancelled = cancellation.is_cancelled();
    if total == skipped && !cancelled {
        diagnostics.error(&anyhow::anyhow!(
            "no video files found in {}",
            template.input.display()
        ));
    }
    diagnostics.batch_summary(total, succeeded, failures.len(), skipped, cancelled);
    RunOutcome::Batch(BatchOutcome {
        total,
        succeeded,
        skipped,
        failures,
        cancelled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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

    #[tokio::test]
    async fn run_with_events_reports_single_task_lifecycle() {
        let cancellation = CancellationToken::new();
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
                        status: TaskStatus::Cancelled,
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
