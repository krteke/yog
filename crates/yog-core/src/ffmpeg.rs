use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

use crate::program::Program;
mod args;
pub mod capabilities;
pub mod decoding;
pub mod encoding;
pub mod execution;
pub mod plan;
pub mod progress;
pub mod vmaf;

pub struct Ffmpeg {
    inner: Program,
}

impl Ffmpeg {
    pub fn new(path: impl Into<PathBuf>, timeout: Option<Duration>) -> Self {
        Self {
            inner: Program {
                path: path.into(),
                timeout,
                cancellation: CancellationToken::new(),
            },
        }
    }

    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.inner.timeout = timeout;
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.inner.cancellation = cancellation;
        self
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.inner.timeout
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.inner.cancellation.clone()
    }
}
