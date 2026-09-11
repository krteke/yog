use std::{ffi::OsString, path::Path};
use tokio::io::AsyncWriteExt;

use super::{Ffmpeg, args::Arg};
use crate::error::{Error, Failure};

impl Ffmpeg {
    pub(super) async fn extract_cover(
        &self,
        input: &Path,
        stream_index: usize,
        path: &Path,
        on_stderr: impl FnMut(&[u8]) + Send,
    ) -> Result<(), Error> {
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
        let output = self
            .inner
            .run(
                args,
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
