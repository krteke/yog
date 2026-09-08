use std::{ffi::OsStr, process::ExitStatus};
use tokio::io::{AsyncBufReadExt, BufReader};

use super::{
    Ffmpeg,
    args::Arg,
    progress::{Progress, ProgressParser},
};
use crate::error::{Error, Failure};

/// Diagnostics retained after successful execution. Failures carry the same
/// diagnostics and exit status in [`Error`].
#[derive(Debug)]
pub struct ExecutionResult {
    pub status: ExitStatus,
    pub stderr: Vec<u8>,
}

impl Ffmpeg {
    /// Execute a caller-planned command, streaming progress and stderr.
    /// Callbacks run in this async task and must return promptly. To cancel and
    /// wait for cleanup, cancel the configured token and await this method.
    /// Dropping the future requests a kill but cannot await process cleanup.
    pub async fn execute<I, O>(
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
        let output = self
            .inner
            .run(
                options,
                |stdout| async move {
                    let mut parser = ProgressParser::new();
                    let mut lines = BufReader::new(stdout).lines();
                    while let Some(line) = lines.next_line().await.map_err(Failure::Io)? {
                        if let Some(progress) = parser.push_line(&line) {
                            on_progress(progress);
                        }
                    }
                    Ok(())
                },
                on_stderr,
            )
            .await?;
        if !output.status.success() {
            return Err(output.failure(Failure::Exit));
        }
        Ok(ExecutionResult {
            status: output.status,
            stderr: output.stderr,
        })
    }
}
