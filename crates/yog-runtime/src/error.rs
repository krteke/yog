use crate::event::{RunPhase, SkipReason, TaskResult};
use yog_core::error::Failure;

#[derive(Debug)]
pub enum RunError {
    Cancelled,
    Failed {
        phase: RunPhase,
        error: anyhow::Error,
    },
    Skipped {
        reason: SkipReason,
        error: anyhow::Error,
    },
}

pub fn timed_out(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<yog_core::error::Error>()
            .is_some_and(|error| matches!(error.reason, Failure::TimedOut))
    })
}

impl RunError {
    /// Bind a failure to its operation at the boundary where it occurs.
    pub fn at(phase: RunPhase, error: anyhow::Error) -> Self {
        if error.chain().any(|cause| {
            cause
                .downcast_ref::<yog_core::error::Error>()
                .is_some_and(|error| matches!(error.reason, Failure::Cancelled))
        }) {
            Self::Cancelled
        } else {
            Self::Failed { phase, error }
        }
    }

    pub fn result(&self) -> TaskResult {
        match self {
            Self::Cancelled => TaskResult::Cancelled,
            Self::Failed { phase, error } => TaskResult::Failed {
                phase: *phase,
                timed_out: timed_out(error),
                message: format!("{error:#}"),
            },
            Self::Skipped { reason, error } => TaskResult::Skipped {
                reason: *reason,
                message: format!("{error:#}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn process_error(reason: Failure) -> anyhow::Error {
        anyhow::Error::new(yog_core::error::Error {
            program: "ffmpeg".into(),
            reason,
            status: None,
            stderr: Vec::new(),
            secondary_io: Vec::new(),
        })
        .context("outer operation")
    }

    #[test]
    fn wrapped_timeout_keeps_its_failure_phase_in_reports_and_diagnostics() {
        let result = RunError::at(RunPhase::Planning, process_error(Failure::TimedOut)).result();
        assert_eq!(result.status(), crate::event::TaskStatus::Failure);
        assert_eq!(result.to_string(), "Failed · Planning · Timed out");
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "status": "failed", "phase": "planning", "timed_out": true,
                "message": "outer operation: ffmpeg: timed out",
            })
        );
    }

    #[test]
    fn timeout_words_in_an_io_error_are_not_a_process_timeout() {
        let result = RunError::at(
            RunPhase::Publishing,
            anyhow::anyhow!("timed out writing filename"),
        )
        .result();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "status": "failed", "phase": "publishing", "timed_out": false,
                "message": "timed out writing filename",
            })
        );
    }

    #[test]
    fn wrapped_process_cancellation_is_not_a_failure() {
        let result =
            RunError::at(RunPhase::Transcoding, process_error(Failure::Cancelled)).result();
        assert_eq!(result, TaskResult::Cancelled);
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({ "status": "cancelled" })
        );
    }
}
