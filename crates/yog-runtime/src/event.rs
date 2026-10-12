use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, time::Duration};

use yog_core::ffmpeg::{prediction::Prediction, progress::Progress, vmaf::VmafOptions};

pub(crate) type EventSink<'a> = dyn Fn(RunEvent) + Send + Sync + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Discovering,
    Probing,
    PreparingOutput,
    Planning,
    Predicting,
    Transcoding,
    Verifying,
    Publishing,
    CalculatingVmaf,
    RenderingChart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Success,
    Failure,
    Skipped,
    Cancelled,
}

impl TaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Success => "Success",
            Self::Failure => "Failed",
            Self::Skipped => "Skipped",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl RunPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Discovering => "Discovering",
            Self::Probing => "Probing",
            Self::PreparingOutput => "Output preparation",
            Self::Planning => "Planning",
            Self::Predicting => "Predicting",
            Self::Transcoding => "Transcoding",
            Self::Verifying => "Verifying",
            Self::Publishing => "Publishing",
            Self::CalculatingVmaf => "VMAF",
            Self::RenderingChart => "Chart rendering",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    NoVideo,
    ProbeFailed,
    ProbeTimedOut,
    UnsupportedContainer,
    AlreadyCompleted,
    ResumeMismatch,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoVideo => "No video",
            Self::ProbeFailed => "Probe failed",
            Self::ProbeTimedOut => "Probe timed out",
            Self::UnsupportedContainer => "Unsupported container",
            Self::AlreadyCompleted => "Already completed",
            Self::ResumeMismatch => "Resume mismatch",
        }
    }
}

/// A terminal result shared by events, reports and frontends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskResult {
    Success,
    Skipped {
        reason: SkipReason,
        message: String,
    },
    Failed {
        phase: RunPhase,
        timed_out: bool,
        message: String,
    },
    Cancelled,
}

impl TaskResult {
    pub fn status(&self) -> TaskStatus {
        match self {
            Self::Success => TaskStatus::Success,
            Self::Skipped { .. } => TaskStatus::Skipped,
            Self::Failed { .. } => TaskStatus::Failure,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Skipped { message, .. } | Self::Failed { message, .. } => Some(message),
            Self::Success | Self::Cancelled => None,
        }
    }
}

impl fmt::Display for TaskResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.status().label())?;
        match self {
            Self::Skipped { reason, .. } => write!(f, " · {}", reason.label()),
            Self::Failed {
                phase, timed_out, ..
            } => {
                write!(f, " · {}", phase.label())?;
                if *timed_out {
                    f.write_str(" · Timed out")?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug)]
pub enum VmafOutcome {
    Scored(f64),
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct RecoveryInfo {
    pub forced: bool,
    pub output: Option<PathBuf>,
}

#[derive(Debug)]
pub enum RunEvent {
    RecoveryPrepared {
        path: PathBuf,
        reused: usize,
        forced: usize,
    },
    PhaseChanged(RunPhase),
    /// The path scan has finished. `total` counts candidate files, not videos;
    /// eligibility is determined by probing each file before execution.
    BatchDiscovered {
        total: usize,
    },
    TaskStarted {
        index: usize,
        total: usize,
        input: PathBuf,
        output: Option<PathBuf>,
    },
    Progress {
        duration: Option<Duration>,
        progress: Progress,
    },
    PredictionCompleted {
        input: PathBuf,
        prediction: Prediction,
    },
    EmulationPointStarted {
        index: usize,
        total: usize,
        candidate: Option<usize>,
        label: String,
        parameter: &'static str,
        quality: u8,
    },
    EmulationPointFinished {
        index: usize,
        total: usize,
        candidate: Option<usize>,
        quality: u8,
        result: TaskResult,
        vmaf: Option<f64>,
        output_bytes: Option<u64>,
    },
    EmulationChartRendered {
        png: Option<PathBuf>,
        svg: Option<PathBuf>,
    },
    VmafFinished {
        output: PathBuf,
        options: VmafOptions,
        outcome: VmafOutcome,
    },
    Warning {
        message: String,
    },
    Error {
        message: String,
    },
    TaskFinished {
        recovery: Option<RecoveryInfo>,
        index: usize,
        total: usize,
        input: PathBuf,
        result: TaskResult,
    },
}
