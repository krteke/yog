use serde::{Deserialize, Serialize};
use std::{fmt, io, path::PathBuf, process::ExitStatus};
use thiserror::Error;

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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProbeValueError {
    #[error("ffprobe did not return {field}")]
    Missing { field: &'static str },
    #[error("ffprobe returned invalid {field} {value:?}")]
    Invalid { field: &'static str, value: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProbeError {
    pub code: i64,
    pub string: String,
}

#[derive(Debug, Error)]
pub enum PredictionError {
    #[error("prediction requires video encoding; stream copy has no useful encoding score")]
    RequiresEncoding,
    #[error("input has no regular video stream")]
    NoVideo,
    #[error("video stream #{stream_index} has no valid display dimensions")]
    MissingVideoDimensions { stream_index: usize },
    #[error("encoded sample has no regular video stream")]
    NoSampleVideo,
    #[error("video stream #{stream_index} does not overlap the input timeline")]
    InvalidVideoSpan { stream_index: usize },
    #[error("attachment stream #{stream_index} has no size")]
    MissingAttachmentSize { stream_index: usize },
    #[error("sample #{sample} produced no timed media packets")]
    NoTimedPackets { sample: usize },
    #[error("byte count overflowed")]
    ByteCountOverflow,
    #[error("sample #{sample} packet payload exceeds its file size")]
    PacketPayloadExceedsFile { sample: usize },
    #[error("libvmaf compared no frames")]
    NoScoredFrames,
    #[error("libvmaf returned no VMAF score")]
    NoVmafScore,
    #[error("predicted output size is outside the supported range")]
    OutputSizeOverflow,
    #[error(transparent)]
    ProbeValue(#[from] ProbeValueError),
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error("command execution failed")]
    Command(#[from] Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
