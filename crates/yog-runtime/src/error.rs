use yog_core::error::Failure;

#[derive(Debug)]
pub enum RunError {
    Cancelled,
    Failed(anyhow::Error),
}

impl From<anyhow::Error> for RunError {
    fn from(error: anyhow::Error) -> Self {
        if error
            .chain()
            .find_map(|cause| cause.downcast_ref::<yog_core::error::Error>())
            .is_some_and(|error| matches!(error.reason, Failure::Cancelled))
        {
            Self::Cancelled
        } else {
            Self::Failed(error)
        }
    }
}
