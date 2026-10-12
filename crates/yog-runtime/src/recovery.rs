mod journal;
pub(crate) use journal::{RecoveredTask, Recovery};

use std::{fmt, path::PathBuf, str::FromStr};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecoveryMode {
    #[default]
    Automatic,
    Restart,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchPolicy {
    Abort,
    Force,
    Skip,
    Restart,
}

impl FromStr for MismatchPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "abort" => Ok(Self::Abort),
            "force" => Ok(Self::Force),
            "skip" => Ok(Self::Skip),
            "restart" => Ok(Self::Restart),
            _ => Err("expected abort, force, skip or restart".into()),
        }
    }
}

impl fmt::Display for MismatchPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Abort => "abort",
            Self::Force => "force",
            Self::Skip => "skip",
            Self::Restart => "restart",
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryOptions {
    pub mode: RecoveryMode,
    pub path: Option<PathBuf>,
    pub batch_mismatch: MismatchPolicy,
    pub file_mismatch: MismatchPolicy,
}

impl Default for RecoveryOptions {
    fn default() -> Self {
        Self {
            mode: RecoveryMode::Automatic,
            path: None,
            batch_mismatch: MismatchPolicy::Abort,
            file_mismatch: MismatchPolicy::Restart,
        }
    }
}
