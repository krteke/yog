use std::{
    io::{self, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

use tokio::task::{JoinHandle, spawn_blocking};
use tokio_util::sync::CancellationToken;

const DIAGNOSTICS_INTERVAL: Duration = Duration::from_millis(10);

pub struct Diagnostics {
    sender: Option<Sender<Vec<u8>>>,
    worker: Option<JoinHandle<()>>,
    stopping: CancellationToken,
    verbose: bool,
    streamed: AtomicBool,
}

impl Diagnostics {
    pub fn new(verbose: bool, cancelled: CancellationToken) -> Self {
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        let stopping = CancellationToken::new();
        let stop = stopping.clone();
        let worker = spawn_blocking(move || {
            for bytes in receiver {
                let mut remaining = bytes.as_slice();
                while !remaining.is_empty() {
                    let result = io::stderr().lock().write(remaining);
                    match result {
                        Ok(0) => return,
                        Ok(count) => remaining = &remaining[count..],
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            if stop.is_cancelled() || cancelled.is_cancelled() {
                                return;
                            }
                            // TODO: ??
                            thread::sleep(DIAGNOSTICS_INTERVAL);
                        }
                        Err(_) => return,
                    }
                }
            }
        });
        Self {
            sender: Some(sender),
            worker: Some(worker),
            stopping,
            verbose,
            streamed: AtomicBool::new(false),
        }
    }

    pub fn write(&self, bytes: &[u8]) {
        let _ = self.sender.as_ref().unwrap().send(bytes.to_owned());
    }

    pub fn ffmpeg(&self, bytes: &[u8]) {
        if self.verbose {
            self.streamed.store(true, Ordering::Relaxed);
            self.write(bytes);
        }
    }

    pub fn error(&self, error: &anyhow::Error) {
        self.write(format!("{error:#}\n").as_bytes());

        let Some(err) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<yog_core::error::Error>())
        else {
            return;
        };
        if !err.stderr.is_empty() {
            if !self.streamed.load(Ordering::Relaxed) {
                self.write(&err.stderr);
            }
            if !err.stderr.ends_with(b"\n") {
                self.write(b"\n");
            }
        }
        if matches!(err.reason, yog_core::error::Failure::TimedOut) {
            self.stopping.cancel();
        }
    }

    pub async fn finish(mut self) {
        self.sender.take();
        if let Err(error) = self.worker.take().unwrap().await {
            std::panic::resume_unwind(error.into_panic());
        }
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.stopping.cancel();
    }
}
