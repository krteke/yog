mod journal;
pub(crate) use journal::{RecoveredTask, Recovery};

use std::{fmt, path::PathBuf, str::FromStr};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecoveryMode {
    #[default]
    Automatic,
    Restart,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchPolicy {
    Ask,
    Abort,
    Force,
    Skip,
    Restart,
}

impl FromStr for MismatchPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ask" => Ok(Self::Ask),
            "abort" => Ok(Self::Abort),
            "force" => Ok(Self::Force),
            "skip" => Ok(Self::Skip),
            "restart" => Ok(Self::Restart),
            _ => Err("expected ask, abort, force, skip or restart".into()),
        }
    }
}

impl fmt::Display for MismatchPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ask => "ask",
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
            batch_mismatch: MismatchPolicy::Ask,
            file_mismatch: MismatchPolicy::Ask,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryScope {
    Batch,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryDifferenceKind {
    InputRoot,
    OutputRoot,
    Container,
    VideoEncoding,
    Decoding,
    SourceFingerprint,
    OutputFingerprint,
}

impl fmt::Display for RecoveryDifferenceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputRoot => "Input directory",
            Self::OutputRoot => "Output directory",
            Self::Container => "Container",
            Self::VideoEncoding => "Video encoding",
            Self::Decoding => "Decoder",
            Self::SourceFingerprint => "Source file",
            Self::OutputFingerprint => "Output file",
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryDifference {
    pub kind: RecoveryDifferenceKind,
    pub recorded: String,
    pub current: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    Abort,
    Force,
    Skip,
    Restart,
}

impl RecoveryAction {
    pub const ALL: [Self; 4] = [Self::Abort, Self::Force, Self::Skip, Self::Restart];
}

#[derive(Debug)]
pub struct RecoveryDecision {
    pub action: RecoveryAction,
    pub apply_to_similar: bool,
}

#[derive(Debug)]
pub struct RecoveryRequest {
    pub state_path: PathBuf,
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub scope: RecoveryScope,
    pub differences: Vec<RecoveryDifference>,
    pub overwrite_output: bool,
    pub reply: oneshot::Sender<Result<RecoveryDecision, String>>,
}

impl RecoveryRequest {
    pub fn respond(self, action: RecoveryAction, apply_to_similar: bool) -> bool {
        self.reply
            .send(Ok(RecoveryDecision {
                action,
                apply_to_similar,
            }))
            .is_ok()
    }

    pub fn fail(self, message: String) -> bool {
        self.reply.send(Err(message)).is_ok()
    }

    pub fn title(&self) -> &'static str {
        if self
            .differences
            .iter()
            .any(|difference| difference.kind == RecoveryDifferenceKind::InputRoot)
        {
            "Input directory changed"
        } else {
            match self.scope {
                RecoveryScope::Batch => "Recovery settings differ",
                RecoveryScope::File => "Recorded files changed",
            }
        }
    }

    pub fn action_label(&self, action: RecoveryAction) -> &'static str {
        match action {
            RecoveryAction::Abort => "Stop batch",
            RecoveryAction::Force
                if self
                    .differences
                    .iter()
                    .any(|difference| difference.kind == RecoveryDifferenceKind::InputRoot) =>
            {
                "Use new input directory"
            }
            RecoveryAction::Force => "Trust recorded results",
            RecoveryAction::Skip => "Skip affected tasks",
            RecoveryAction::Restart => match self.scope {
                RecoveryScope::Batch => "Restart batch",
                RecoveryScope::File => "Redo file",
            },
        }
    }

    pub fn action_detail(&self, action: RecoveryAction) -> &'static str {
        match action {
            RecoveryAction::Abort => "Stop without changing the affected files or saved state.",
            RecoveryAction::Force
                if self
                    .differences
                    .iter()
                    .any(|difference| difference.kind == RecoveryDifferenceKind::InputRoot) =>
            {
                "Use the new input directory. Individual source and output fingerprints are still checked."
            }
            RecoveryAction::Force if self.scope == RecoveryScope::Batch => {
                "Reuse eligible results made with the recorded settings. Files are still checked; completed videos stay at their recorded paths."
            }
            RecoveryAction::Force => {
                "Trust eligible records despite these differences. Missing outputs are not regenerated; completed videos stay at their recorded paths."
            }
            RecoveryAction::Skip => {
                "Skip affected tasks in this run without replacing their historical records."
            }
            RecoveryAction::Restart if self.overwrite_output => {
                "Run again with current settings. Output overwrite is enabled: existing videos may be replaced."
            }
            RecoveryAction::Restart => {
                "Run again with current settings. Existing videos will not be overwritten."
            }
        }
    }
}
