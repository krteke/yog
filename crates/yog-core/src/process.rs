use crate::error::{Error, Failure};
use std::{path::PathBuf, process::ExitStatus};

pub struct Output<T> {
    pub program: PathBuf,
    pub status: ExitStatus,
    pub value: T,
    pub stderr: Vec<u8>,
}

impl<T> Output<T> {
    pub fn failure(self, reason: Failure) -> Error {
        Error {
            program: self.program,
            reason,
            status: Some(self.status),
            stderr: self.stderr,
            secondary_io: Vec::new(),
        }
    }
}
