use tokio::process::ChildStdout;
use tokio_util::{io::SyncIoBridge, sync::CancellationToken};
mod args;
mod streaming;
pub mod types;

use crate::{
    error::{Error, Failure},
    program::Program,
};
use args::{Arg, Entries};
use serde::Deserialize;
use std::{
    io::BufReader,
    path::{Path, PathBuf},
    time::Duration,
};
use types::{Chapter, Frame, MediaFormat, MediaInfo, MediaStream, Packet, ProbeError};

#[derive(Debug)]
pub struct ProbeResult<T> {
    pub output: T,
    pub stderr: Vec<u8>,
}

#[derive(Deserialize)]
struct MediaResponse {
    #[serde(default)]
    streams: Vec<MediaStream>,
    #[serde(default)]
    chapters: Vec<Chapter>,
    #[serde(default)]
    programs: Vec<serde_json::Value>,
    #[serde(default)]
    format: MediaFormat,
    error: Option<ProbeError>,
}

#[derive(Debug, Clone)]
pub struct Ffprobe {
    inner: Program,
}

impl Ffprobe {
    pub fn new(path: impl Into<PathBuf>, timeout: Option<Duration>) -> Self {
        Self {
            inner: Program {
                path: path.into(),
                timeout,
                cancellation: CancellationToken::new(),
            },
        }
    }

    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.inner.timeout = timeout;
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.inner.cancellation = cancellation;
        self
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.inner.timeout
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.inner.cancellation.clone()
    }

    pub async fn probe(&self, input: &Path) -> Result<ProbeResult<MediaInfo>, Error> {
        self.query(
            input,
            [
                Arg::ShowFormat,
                Arg::ShowStreams,
                Arg::ShowChapters,
                Arg::ShowPrograms,
            ],
            move |reader| {
                let response: MediaResponse =
                    serde_json::from_reader(reader).map_err(Failure::Json)?;
                Ok((
                    MediaInfo {
                        streams: response.streams,
                        chapters: response.chapters,
                        programs: response.programs,
                        format: response.format,
                    },
                    response.error,
                ))
            },
        )
        .await
    }

    /// Delivers frames in a blocking parser task. Records are provisional until this
    /// method returns Ok; cancellation/failure can follow already delivered frames.
    /// The callback must return promptly so cancellation can finish waiting for it.
    /// It must own its captures (`Send + 'static`); use a channel or shared state
    /// to return records to the caller. Cancelling the token and awaiting this
    /// method completes cleanup; dropping the future only requests a kill.
    pub async fn frames(
        &self,
        input: &Path,
        stream_index: usize,
        read_intervals: &str,
        mut consume: impl FnMut(Frame) + Send + 'static,
    ) -> Result<ProbeResult<()>, Error> {
        self.query(
            input,
            [
                Arg::SelectStream(stream_index),
                Arg::ReadIntervals(read_intervals),
                Arg::ShowFrames,
                Arg::ShowEntries(Entries::Frame),
            ],
            move |reader| {
                let error =
                    streaming::records(reader, "frames", &mut consume).map_err(Failure::Json)?;
                Ok(((), error))
            },
        )
        .await
    }

    /// Streams packets without collecting them. None selects all streams.
    /// As with frames(), callbacks precede the final success/failure result.
    pub async fn packets(
        &self,
        input: &Path,
        stream_index: Option<usize>,
        read_intervals: &str,
        mut consume: impl FnMut(Packet) + Send + 'static,
    ) -> Result<ProbeResult<()>, Error> {
        let args = [
            Arg::ReadIntervals(read_intervals),
            Arg::ShowPackets,
            Arg::ShowEntries(Entries::Packet),
        ]
        .into_iter()
        .chain(stream_index.map(Arg::SelectStream));
        self.query(input, args, move |reader| {
            let error =
                streaming::records(reader, "packets", &mut consume).map_err(Failure::Json)?;
            Ok(((), error))
        })
        .await
    }

    async fn query<'a, I, T, D>(
        &self,
        input: &Path,
        options: I,
        decode: D,
    ) -> Result<ProbeResult<T>, Error>
    where
        T: Send + 'static,
        I: IntoIterator<Item = Arg<'a>>,
        D: FnOnce(BufReader<SyncIoBridge<ChildStdout>>) -> Result<(T, Option<ProbeError>), Failure>
            + Send
            + 'static,
    {
        let mut args = Vec::new();
        for option in [Arg::ErrorsOnly, Arg::Json, Arg::ShowError]
            .into_iter()
            .chain(options)
        {
            option.append_to(&mut args);
        }
        Arg::Input(input).append_to(&mut args);
        let mut output = self
            .inner
            .run(
                &args,
                |stdout| async move {
                    let reader = BufReader::new(SyncIoBridge::new(stdout));
                    match tokio::task::spawn_blocking(move || decode(reader)).await {
                        Ok(result) => result,
                        Err(error) => std::panic::resume_unwind(error.into_panic()),
                    }
                },
                |_| {},
            )
            .await?;
        if let Some(error) = output.value.1.take() {
            return Err(output.failure(Failure::Probe(error)));
        }
        if !output.status.success() {
            return Err(output.failure(Failure::Exit));
        }
        Ok(ProbeResult {
            output: output.value.0,
            stderr: output.stderr,
        })
    }
}
