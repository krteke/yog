use std::process::ExitStatus;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::{
    Ffmpeg,
    args::{Arg, ArgsExt},
    attachment::CoverExtraction,
    plan::{AttachmentInput, TranscodePlan},
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
    attachment_dir_guard: Option<tempfile::TempDir>,
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

        let attachment_dir = if plan.attachments.is_empty() {
            None
        } else {
            Some(
                tempfile::Builder::new()
                    .prefix("yog-attachments-")
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
        let mut paths = Vec::new();
        if let Some(dir) = &attachment_dir {
            for attachment in &plan.attachments {
                let path = dir
                    .path()
                    .join(format!("attachment-{}", attachment.input_index));
                match attachment.input {
                    AttachmentInput::AttachmentStream => {
                        args.add(Arg::DumpAttachment(attachment.input_index, &path));
                    }
                    AttachmentInput::CoverFrame => {
                        covers.push(self.build_cover_extraction(
                            &plan.input,
                            attachment.input_index,
                            &path,
                        ));
                    }
                }
                paths.push(path);
            }
        }

        args.extend_from_slice(&plan.args);
        for (attachment, path) in plan.attachments.iter().zip(&paths) {
            args.add(Arg::Attach(path));
            args.extend(
                attachment
                    .metadata
                    .iter()
                    .map(|(key, value)| Arg::StreamMetadata(attachment.output_index, key, value)),
            );
        }
        args.add(Arg::Output(&plan.output));

        Ok(BuiltTranscode {
            command: self.inner.build(args),
            covers,
            attachment_dir_guard: attachment_dir,
        })
    }
}

impl BuiltTranscode<'_> {
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
            attachment_dir_guard: _guard,
        } = self;

        for extraction in covers {
            extraction.run(&mut on_stderr).await?;
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
