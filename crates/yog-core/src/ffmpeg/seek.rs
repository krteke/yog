use std::{ffi::OsString, path::Path};

use tokio::io::{AsyncBufReadExt, BufReader};

use super::{Ffmpeg, args::Arg};
use crate::error::{Error, Failure};

#[derive(Debug)]
pub struct SeekDecodeResult {
    pub audio_frames: u64,
    pub stderr: Vec<u8>,
}

impl Ffmpeg {
    pub async fn seek_decode_audio(
        &self,
        input: &Path,
        video_index: usize,
        audio_index: usize,
        timestamp: &str,
    ) -> Result<SeekDecodeResult, Error> {
        let mut args: Vec<OsString> = Vec::new();
        args.extend([
            Arg::HideBanner,
            Arg::NoStdin,
            Arg::LogLevel("error"),
            Arg::Seek(timestamp),
            Arg::Input(input),
            Arg::Map(video_index),
            Arg::Map(audio_index),
            Arg::CopyStream(0),
            Arg::Duration("1"),
            Arg::Format("framehash"),
            Arg::Stdout,
        ]);
        let command = self.inner.build(args);

        let output = command
            .run(
                |stdout| async move {
                    let mut frames = 0;
                    let mut lines = BufReader::new(stdout).lines();
                    while let Some(line) = lines.next_line().await.map_err(Failure::Io)? {
                        if line
                            .split_once(',')
                            .is_some_and(|(stream, _)| stream.trim() == "1")
                        {
                            frames += 1;
                        }
                    }
                    Ok(frames)
                },
                |_| {},
            )
            .await?;
        if !output.status.success() {
            return Err(output.failure(Failure::Exit));
        }

        Ok(SeekDecodeResult {
            audio_frames: output.value,
            stderr: output.stderr,
        })
    }
}
