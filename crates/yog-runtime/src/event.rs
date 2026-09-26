use std::{path::PathBuf, time::Duration};

use yog_core::ffmpeg::{prediction::Prediction, progress::Progress, vmaf::VmafOptions};

pub(crate) type EventSink<'a> = dyn Fn(RunEvent) + Send + Sync + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPhase {
    Discovering,
    Probing,
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
    Cancelled,
}

#[derive(Debug)]
pub enum VmafOutcome {
    Scored(f64),
    Failed(String),
    Cancelled,
}

#[derive(Debug)]
pub enum RunEvent {
    PhaseChanged(RunPhase),
    BatchDiscovered {
        total: usize,
        skipped: usize,
    },
    InputSkipped {
        input: PathBuf,
        error: String,
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
        status: TaskStatus,
        vmaf: Option<f64>,
        output_bytes: Option<u64>,
        error: Option<String>,
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
        index: usize,
        total: usize,
        input: PathBuf,
        status: TaskStatus,
        error: Option<String>,
    },
}
