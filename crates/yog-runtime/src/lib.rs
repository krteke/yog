mod config;
mod diagnostics;
mod emulate;
mod error;
mod output;
mod progress;
mod recursive;
mod transcode;
mod validate;
mod verify;

use diagnostics::Diagnostics;
use error::RunError;
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;
use transcode::Transcoder;
use yog_core::ffmpeg::{plan::TranscodeRequest, vmaf::VmafOptions};

pub use config::Config;
pub use emulate::EmulationOptions;

#[derive(Debug)]
pub struct Command {
    pub request: TranscodeRequest,
    pub operation: Operation,
    pub recursive: bool,
}

impl Command {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate::command(self)
    }
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
    /// Controls direct diagnostics and progress output. Log records still use the `log` facade.
    pub terminal_output: bool,
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

pub async fn run(
    command: Command,
    options: Options,
    config: Config,
    cancellation: CancellationToken,
) -> RunOutcome {
    let diagnostics = Diagnostics::new(options.verbose, options.terminal_output);
    if let Err(error) = command.validate() {
        diagnostics.error(&error);
        return RunOutcome::Failed(error);
    }

    let transcoder = Transcoder::new(&options, config, cancellation.clone());
    if command.recursive {
        return run_batch(command, &transcoder, &diagnostics, &cancellation).await;
    }

    let result = match &command.operation {
        Operation::Transcode => transcoder.run(command.request, &diagnostics).await,
        Operation::Predict => transcoder.predict(command.request, &diagnostics).await,
        Operation::Emulate(options) => {
            transcoder
                .emulate(command.request, options, &diagnostics)
                .await
        }
    };

    match result {
        Ok(()) => RunOutcome::Completed,
        Err(RunError::Cancelled) => {
            diagnostics.cancelled();
            RunOutcome::Cancelled
        }
        Err(RunError::Failed(error)) => {
            diagnostics.error(&error);
            RunOutcome::Failed(error)
        }
    }
}

async fn run_batch(
    command: Command,
    transcoder: &Transcoder,
    diagnostics: &Diagnostics,
    cancellation: &CancellationToken,
) -> RunOutcome {
    let predict = matches!(&command.operation, Operation::Predict);
    let tasks = match recursive::discover(command.request, transcoder, diagnostics, predict).await {
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
    let mut succeeded = 0;
    let mut failures = Vec::new();

    for (request, media) in tasks {
        let input = request.input.clone();
        diagnostics.begin_task();
        let result = match &command.operation {
            Operation::Predict => transcoder.predict_probed(request, media, diagnostics).await,
            Operation::Transcode => transcoder.run_probed(request, media, diagnostics).await,
            Operation::Emulate(_) => unreachable!("validated recursive command cannot emulate"),
        };
        match result {
            Ok(()) => succeeded += 1,
            Err(RunError::Failed(error)) => {
                diagnostics.task_error(&input, &error);
                failures.push(TaskFailure { input, error });
            }
            Err(RunError::Cancelled) => break,
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
