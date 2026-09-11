use std::{fmt, io, path::PathBuf, process::ExitStatus};
use thiserror::Error;

use crate::ffprobe::types::ProbeError;

#[derive(Debug)]
pub enum Failure {
    Io(io::Error),
    Json(serde_json::Error),
    Cancelled,
    TimedOut,
    Exit,
    Probe(ProbeError),
    InvalidOutput(&'static str),
}

/// The primary failure plus diagnostics collected before the process was reaped.
#[derive(Debug, Error)]
pub struct Error {
    pub program: PathBuf,
    pub reason: Failure,
    pub status: Option<ExitStatus>,
    pub stderr: Vec<u8>,
    /// Additional errors encountered reading diagnostics or killing/waiting.
    pub secondary_io: Vec<io::Error>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.program.display())?;
        match &self.reason {
            Failure::Io(e) => write!(f, "{e}"),
            Failure::Json(e) => write!(f, "invalid JSON: {e}"),
            Failure::Cancelled => f.write_str("cancelled"),
            Failure::TimedOut => f.write_str("timed out"),
            Failure::Exit => f.write_str("process failed"),
            Failure::Probe(e) => write!(f, "ffprobe error {}: {}", e.code, e.string),
            Failure::InvalidOutput(message) => f.write_str(message),
        }?;
        if let Some(status) = self.status {
            write!(f, " ({status})")?;
        }
        for error in &self.secondary_io {
            write!(f, "; additional I/O failure: {error}")?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("encoder format query failed")]
    Query(#[from] Error),
    #[error("cannot use input format {format:?} as an output container; specify a container")]
    UnsupportedContainer { format: Option<String> },
    #[error("stream {stream_index}: ffprobe has no descriptor for pixel format {format:?}")]
    MissingPixelFormat {
        stream_index: usize,
        format: Option<String>,
    },
}
