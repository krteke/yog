use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;

use super::{Ffmpeg, args::Arg};
use crate::{
    error::{Error, Failure},
    program::Command,
};

pub(super) struct CoverExtraction<'a> {
    command: Command<'a>,
    path: PathBuf,
}

impl Ffmpeg {
    pub(super) fn build_cover_extraction<'a>(
        &'a self,
        input: &Path,
        stream_index: usize,
        path: &Path,
    ) -> CoverExtraction<'a> {
        let mut args: Vec<OsString> = Vec::new();
        args.extend([
            Arg::HideBanner,
            Arg::NoStdin,
            Arg::NoStats,
            Arg::Input(input),
            Arg::Map(stream_index),
            Arg::CopyAll,
            Arg::OneVideoFrame,
            Arg::Format("image2pipe"),
            Arg::ImageStdout,
        ]);
        CoverExtraction {
            command: self.inner.build(args),
            path: path.to_owned(),
        }
    }
}

impl CoverExtraction<'_> {
    pub(super) fn print(&self) {
        self.command.print();
    }

    pub(super) async fn run(self, on_stderr: impl FnMut(&[u8]) + Send) -> Result<(), Error> {
        let path = self.path;
        let output = self
            .command
            .run(
                |mut stdout| async move {
                    let mut file = tokio::fs::File::create(path).await.map_err(Failure::Io)?;
                    let size = tokio::io::copy(&mut stdout, &mut file)
                        .await
                        .map_err(Failure::Io)?;
                    if size == 0 {
                        return Err(Failure::InvalidOutput(
                            "cover extraction produced no image data",
                        ));
                    }
                    file.flush().await.map_err(Failure::Io)
                },
                on_stderr,
            )
            .await?;
        if !output.status.success() {
            return Err(output.failure(Failure::Exit));
        }
        Ok(())
    }
}
