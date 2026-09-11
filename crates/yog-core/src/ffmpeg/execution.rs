use std::process::ExitStatus;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::{
    Ffmpeg,
    args::{Arg, ArgsExt},
    plan::TranscodePlan,
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
    /// Execute the complete plan, including Matroska cover extraction/attachment.
    /// Cover files belong to a TempDir held until the child has been reaped.
    /// The configured timeout applies separately to each FFmpeg subprocess.
    /// Callbacks run in this async task and must return promptly. To cancel and
    /// wait for cleanup, cancel the configured token and await this method.
    /// Dropping the future requests a kill but cannot await process cleanup.
    pub async fn execute(
        &self,
        plan: &TranscodePlan,
        mut on_progress: impl FnMut(Progress) + Send,
        mut on_stderr: impl FnMut(&[u8]) + Send,
    ) -> Result<ExecutionResult, Error> {
        let mut args = Vec::new();
        args.extend([
            Arg::HideBanner,
            Arg::NoStdin,
            Arg::NoStats,
            Arg::ProgressStdout,
        ]);
        args.extend_from_slice(&plan.args);
        let cover_dir = if plan.covers.is_empty() {
            None
        } else {
            Some(
                tempfile::Builder::new()
                    .prefix("yog-covers-")
                    .tempdir()
                    .map_err(|error| Error {
                        program: self.inner.path.clone(),
                        reason: Failure::Io(error),
                        status: None,
                        stderr: Vec::new(),
                        secondary_io: Vec::new(),
                    })?,
            )
        };
        if let Some(dir) = &cover_dir {
            for cover in &plan.covers {
                let path = dir.path().join(format!("cover-{}", cover.input_index));
                self.extract_cover(&plan.input, cover.input_index, &path, &mut on_stderr)
                    .await?;
                args.add(Arg::Attach(&path));
                args.extend(
                    cover
                        .metadata
                        .iter()
                        .map(|(key, value)| Arg::StreamMetadata(cover.output_index, key, value)),
                );
            }
        }
        args.add(Arg::Output(&plan.output));
        let output = self
            .inner
            .run(
                args,
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
