use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    time::Duration,
};

use gpui_kit::BackgroundExecutor;
use tokio_util::sync::CancellationToken;
use yog_runtime::{
    Command, Config, Options, RunOutcome, RunStatus,
    event::{RunEvent, RunPhase},
};

use super::{
    logging,
    messages::{MessageLevel, MessageSource, NewMessage},
};

const QUEUE_CAPACITY: usize = 256;
const MESSAGES_PER_BATCH: usize = 64;
const RECEIVE_INTERVAL: Duration = Duration::from_millis(50);

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

#[derive(Clone)]
pub(super) struct WorkerSender {
    sender: SyncSender<WorkerMessage>,
    overflow: Arc<Mutex<LogOverflow>>,
}

#[derive(Default)]
struct LogOverflow {
    dropped: usize,
    latest: Option<NewMessage>,
}

impl WorkerSender {
    pub fn log(&self, message: NewMessage) {
        let mut overflow = self.overflow.lock().unwrap();
        if let Some(latest) = overflow.latest.take() {
            match self.sender.try_send(WorkerMessage::Log(latest)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => overflow.dropped += 1,
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        if let Err(TrySendError::Full(WorkerMessage::Log(message))) =
            self.sender.try_send(WorkerMessage::Log(message))
        {
            // Preserve the newest diagnostic, which can contain the final error.
            overflow.latest = Some(message);
        }
    }

    fn send(&self, message: WorkerMessage) {
        let latest = self.overflow.lock().unwrap().latest.take();
        // Do not hold the overflow lock while waiting for the GUI to consume.
        if let Some(latest) = latest {
            _ = self.sender.send(WorkerMessage::Log(latest));
        }
        _ = self.sender.send(message);
    }
}

pub struct WorkerReceiver {
    receiver: Receiver<WorkerMessage>,
    overflow: Arc<Mutex<LogOverflow>>,
    disconnected: bool,
}

pub(super) fn channel() -> (WorkerSender, WorkerReceiver) {
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let overflow = Arc::new(Mutex::new(LogOverflow::default()));
    (
        WorkerSender {
            sender,
            overflow: overflow.clone(),
        },
        WorkerReceiver {
            receiver,
            overflow,
            disconnected: false,
        },
    )
}

impl WorkerReceiver {
    pub async fn recv(&mut self, executor: &BackgroundExecutor) -> Option<Vec<WorkerMessage>> {
        loop {
            // Yield between batches even when the worker produces continuously.
            executor.timer(RECEIVE_INTERVAL).await;
            if let Some(batch) = self.next_batch() {
                return Some(batch);
            }
            if self.disconnected {
                return None;
            }
        }
    }

    pub(super) fn next_batch(&mut self) -> Option<Vec<WorkerMessage>> {
        let mut batch = Vec::with_capacity(MESSAGES_PER_BATCH + 1);
        let mut overflow = self.overflow.lock().unwrap();
        for _ in 0..MESSAGES_PER_BATCH {
            match self.receiver.try_recv() {
                Ok(message) => batch.push(message),
                Err(error) => {
                    self.disconnected = error == TryRecvError::Disconnected;
                    if let Some(latest) = overflow.latest.take() {
                        batch.push(WorkerMessage::Log(latest));
                    }
                    break;
                }
            }
        }
        let dropped = std::mem::take(&mut overflow.dropped);
        if dropped > 0 {
            batch.insert(0, WorkerMessage::Log(NewMessage {
                source: MessageSource::Program,
                level: MessageLevel::Warning,
                text: format!("Dropped {dropped} log chunks because messages arrived faster than the panel could consume them"),
            }));
        }
        (!batch.is_empty()).then_some(batch)
    }
}

pub struct ActivityChanged;

pub enum Activity {
    Running(String),
    Waiting(String),
    Cancelling,
    Finished(RunStatus),
}

pub fn phase_label(phase: Option<RunPhase>) -> &'static str {
    match phase {
        Some(RunPhase::Discovering) => "Discovering files",
        Some(RunPhase::Probing) => "Probing media",
        Some(RunPhase::PreparingOutput) => "Preparing output",
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

pub fn spawn(request: RunRequest, cancellation: CancellationToken) -> WorkerReceiver {
    let (sender, receiver) = channel();
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
                    // Lifecycle events must be delivered in order. Only the
                    // dedicated worker thread waits for this bounded queue;
                    // log callbacks use try_send and never block stderr draining.
                    sender.send(WorkerMessage::Event(event));
                },
            )),
            Err(error) => RunOutcome::Failed(error.into()),
        };
        drop(capture);
        sender.send(WorkerMessage::Finished(outcome));
    });
    receiver
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, time::Instant};
    use yog_core::ffmpeg::plan::TranscodeRequest;
    use yog_runtime::{Operation, event::TaskStatus};

    #[test]
    fn stalled_gui_does_not_lose_progress_or_completion_events() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("input.mkv");
        let output = directory.path().join("output.mkv");
        fs::write(&input, b"video").unwrap();
        let ffprobe = directory.path().join("ffprobe");
        fs::write(&ffprobe, "#!/bin/sh\nprintf '%s' '{\"streams\":[{\"index\":0,\"codec_type\":\"audio\"}],\"format\":{\"format_name\":\"matroska,webm\",\"duration\":\"1\"}}'\n").unwrap();
        fs::set_permissions(&ffprobe, fs::Permissions::from_mode(0o700)).unwrap();
        let ffmpeg = directory.path().join("ffmpeg");
        fs::write(&ffmpeg, "#!/bin/sh\ni=0\nwhile [ $i -lt 512 ]; do printf 'frame=%s\\nprogress=continue\\n' \"$i\"; i=$((i+1)); done\nfor last do :; done\nprintf encoded > \"$last\"\nprintf 'frame=512\\nprogress=end\\n'\n").unwrap();
        fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o700)).unwrap();

        let mut receiver = spawn(
            RunRequest {
                command: Command {
                    request: TranscodeRequest::mkv(&input, &output),
                    operation: Operation::Transcode,
                    recursive: false,
                },
                options: Options {
                    ffprobe,
                    ffmpeg,
                    terminal_output: false,
                    ..Options::default()
                },
                config: Config::default(),
            },
            CancellationToken::new(),
        );
        // Deliberately let the worker fill the queue before the GUI consumes it.
        std::thread::sleep(Duration::from_millis(100));
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut progress = 0;
        let mut task_finished = false;
        let outcome = 'receive: loop {
            assert!(
                Instant::now() < deadline,
                "worker did not finish after the GUI resumed"
            );
            if let Some(batch) = receiver.next_batch() {
                for message in batch {
                    match message {
                        WorkerMessage::Event(RunEvent::Progress { .. }) => progress += 1,
                        WorkerMessage::Event(RunEvent::TaskFinished { result, .. }) => {
                            assert_eq!(result.status(), TaskStatus::Success);
                            task_finished = true;
                        }
                        WorkerMessage::Finished(outcome) => break 'receive outcome,
                        _ => {}
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        assert_eq!(progress, 513);
        assert!(task_finished);
        assert!(matches!(outcome, RunOutcome::Completed));
        assert_eq!(fs::read(&output).unwrap(), b"encoded");
    }
}
