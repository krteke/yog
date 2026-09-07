use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
mod args;
pub mod capabilities;
pub mod execution;
pub mod progress;
pub mod vmaf;

pub struct Ffmpeg {
    program: PathBuf,
    timeout: Duration,
    cancellation: Option<Arc<AtomicBool>>,
}

impl Ffmpeg {
    pub fn new(program: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            timeout,
            cancellation: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_cancellation(mut self, cancellation: Option<Arc<AtomicBool>>) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn cancellation(&self) -> Option<Arc<AtomicBool>> {
        self.cancellation.clone()
    }
}
