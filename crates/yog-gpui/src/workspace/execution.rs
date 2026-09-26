use std::path::Path;

use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio_util::sync::CancellationToken;
use yog_runtime::{
    Command, Config, Options, RunOutcome, RunStatus,
    event::{RunEvent, RunPhase},
};

use super::{logging, messages::NewMessage};

pub struct RunRequest {
    pub command: Command,
    pub options: Options,
    pub config: Config,
}

pub enum WorkerMessage {
    Event(RunEvent),
    Log(NewMessage),
    Finished(RunOutcome),
}

pub struct ActivityChanged;

pub enum Activity {
    Running(String),
    Cancelling,
    Finished(RunStatus),
}

pub fn phase_label(phase: Option<RunPhase>) -> &'static str {
    match phase {
        Some(RunPhase::Discovering) => "Discovering files",
        Some(RunPhase::Probing) => "Probing media",
        Some(RunPhase::Planning) => "Preparing run",
        Some(RunPhase::Predicting) => "Predicting",
        Some(RunPhase::Transcoding) => "Transcoding",
        Some(RunPhase::Verifying) => "Verifying output",
        Some(RunPhase::Publishing) => "Publishing output",
        Some(RunPhase::CalculatingVmaf) => "Calculating VMAF",
        Some(RunPhase::RenderingChart) => "Rendering chart",
        None => "Starting",
    }
}

pub fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    let resolve = |path: &Path| {
        path.canonicalize()
            .or_else(|_| std::path::absolute(path))
            .ok()
    };
    matches!((resolve(left), resolve(right)), (Some(left), Some(right)) if left == right)
}

pub fn spawn(
    request: RunRequest,
    cancellation: CancellationToken,
) -> UnboundedReceiver<WorkerMessage> {
    let (sender, receiver) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let capture = logging::capture(sender.clone());
        let outcome = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(yog_runtime::run_with_events(
                request.command,
                request.options,
                request.config,
                cancellation,
                |event| {
                    _ = sender.send(WorkerMessage::Event(event));
                },
            )),
            Err(error) => RunOutcome::Failed(error.into()),
        };
        drop(capture);
        _ = sender.send(WorkerMessage::Finished(outcome));
    });
    receiver
}
