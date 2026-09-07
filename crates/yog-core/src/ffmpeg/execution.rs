use std::{ffi::OsStr, io::BufRead, process::ExitStatus};

use super::{
    Ffmpeg,
    args::Arg,
    progress::{Progress, ProgressParser},
};
use crate::{
    error::{Error, Failure},
    process,
};

/// Diagnostics retained after successful execution. Failures carry the same
/// diagnostics and exit status in [`Error`].
#[derive(Debug)]
pub struct ExecutionResult {
    pub status: ExitStatus,
    pub stderr: Vec<u8>,
}

impl Ffmpeg {
    /// Execute a caller-planned command, streaming progress and stderr.
    pub fn execute<I, O>(
        &self,
        args: I,
        mut on_progress: impl FnMut(Progress) + Send,
        on_stderr: impl FnMut(&[u8]) + Send,
    ) -> Result<ExecutionResult, Error>
    where
        I: IntoIterator<Item = O>,
        O: AsRef<OsStr>,
    {
        let mut options = Vec::new();
        for option in [
            Arg::HideBanner,
            Arg::NoStdin,
            Arg::NoStats,
            Arg::ProgressStdout,
        ] {
            option.append_to(&mut options);
        }
        options.extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        let output = process::run(
            &self.program,
            options,
            self.timeout,
            self.cancellation.clone(),
            |mut reader| {
                let mut parser = ProgressParser::new();
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).map_err(Failure::Io)? == 0 {
                        break;
                    }
                    if let Some(progress) = parser.push_line(&line) {
                        on_progress(progress);
                    }
                }
                Ok(())
            },
            on_stderr,
        )?;
        if !output.status.success() {
            return Err(output.failure(Failure::Exit));
        }
        Ok(ExecutionResult {
            status: output.status,
            stderr: output.stderr,
        })
    }
}
