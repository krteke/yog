use std::process::ExitStatus;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::{
    Ffmpeg,
    args::{Arg, ArgsExt},
    attachment::CoverExtraction,
    plan::TranscodePlan,
    progress::{Progress, ProgressParser},
};
use crate::{
    error::{Error, Failure},
    program::Command,
};

/// Diagnostics retained after successful execution. Failures carry the same
/// diagnostics and exit status in [`Error`].
#[derive(Debug)]
pub struct ExecutionResult {
    pub status: ExitStatus,
    pub stderr: Vec<u8>,
}

pub struct BuiltTranscode<'a> {
    command: Command<'a>,
    covers: Vec<CoverExtraction<'a>>,
    cover_dir: Option<tempfile::TempDir>,
}

impl Ffmpeg {
    pub fn build<'a>(&'a self, plan: &TranscodePlan) -> Result<BuiltTranscode<'a>, Error> {
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
        let mut covers = Vec::new();
        if let Some(dir) = &cover_dir {
            for cover in &plan.covers {
                let path = dir.path().join(format!("cover-{}", cover.input_index));
                covers.push(self.build_cover_extraction(&plan.input, cover.input_index, &path));
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

        Ok(BuiltTranscode {
            command: self.inner.build(args),
            covers,
            cover_dir,
        })
    }
}

impl BuiltTranscode<'_> {
    pub fn print(&self) {
        for cover in &self.covers {
            cover.print();
        }
        self.command.print();
    }

    /// The configured timeout applies separately to each FFmpeg subprocess.
    /// Callbacks run in this async task and must return promptly. To cancel and
    /// wait for cleanup, cancel the configured token and await this method.
    /// Dropping the future requests a kill but cannot await process cleanup.
    pub async fn run(
        self,
        mut on_progress: impl FnMut(Progress) + Send,
        mut on_stderr: impl FnMut(&[u8]) + Send,
    ) -> Result<ExecutionResult, Error> {
        let Self {
            command,
            covers,
            cover_dir: _cover_dir,
        } = self;
        for cover in covers {
            cover.run(&mut on_stderr).await?;
        }

        let output = command
            .run(
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
