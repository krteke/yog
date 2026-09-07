mod args;
mod streaming;
pub mod types;

use crate::{
    error::{Error, Failure},
    process::run,
};
use args::{Arg, Entries};
use serde::Deserialize;
use std::{
    io::BufReader,
    path::{Path, PathBuf},
    process::ChildStdout,
    sync::{Arc, atomic::AtomicBool},
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
    program: PathBuf,
    timeout: Duration,
    cancellation: Option<Arc<AtomicBool>>,
}

impl Ffprobe {
    pub fn new(program: impl Into<PathBuf>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            timeout,
            cancellation: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_cancellation(mut self, cancellation: Option<Arc<AtomicBool>>) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn cancellation(&self) -> Option<Arc<AtomicBool>> {
        self.cancellation.clone()
    }

    pub fn probe(&self, input: &Path) -> Result<ProbeResult<MediaInfo>, Error> {
        self.query(
            input,
            [
                Arg::ShowFormat,
                Arg::ShowStreams,
                Arg::ShowChapters,
                Arg::ShowPrograms,
            ],
            |reader| {
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
    }

    /// Delivers frames on the reader thread. Records are provisional until this
    /// method returns Ok; cancellation/failure can follow already delivered frames.
    /// The callback must return promptly so that cancellation can finish joining it.
    pub fn frames(
        &self,
        input: &Path,
        stream_index: usize,
        read_intervals: &str,
        mut consume: impl FnMut(Frame) + Send,
    ) -> Result<ProbeResult<()>, Error> {
        self.query(
            input,
            [
                Arg::SelectStream(stream_index),
                Arg::ReadIntervals(read_intervals),
                Arg::ShowFrames,
                Arg::ShowEntries(Entries::Frame),
            ],
            |reader| {
                let error =
                    streaming::records(reader, "frames", &mut consume).map_err(Failure::Json)?;
                Ok(((), error))
            },
        )
    }

    /// Streams packets without collecting them. None selects all streams.
    /// As with frames(), callbacks precede the final success/failure result.
    pub fn packets(
        &self,
        input: &Path,
        stream_index: Option<usize>,
        read_intervals: &str,
        mut consume: impl FnMut(Packet) + Send,
    ) -> Result<ProbeResult<()>, Error> {
        let args = [
            Arg::ReadIntervals(read_intervals),
            Arg::ShowPackets,
            Arg::ShowEntries(Entries::Packet),
        ]
        .into_iter()
        .chain(stream_index.map(Arg::SelectStream));
        self.query(input, args, |reader| {
            let error =
                streaming::records(reader, "packets", &mut consume).map_err(Failure::Json)?;
            Ok(((), error))
        })
    }

    fn query<'a, I, T, D>(
        &self,
        input: &Path,
        options: I,
        decode: D,
    ) -> Result<ProbeResult<T>, Error>
    where
        T: Send,
        I: IntoIterator<Item = Arg<'a>>,
        D: FnOnce(BufReader<ChildStdout>) -> Result<(T, Option<ProbeError>), Failure> + Send,
    {
        let mut args = Vec::new();
        for option in [Arg::ErrorsOnly, Arg::Json, Arg::ShowError]
            .into_iter()
            .chain(options)
        {
            option.append_to(&mut args);
        }
        Arg::Input(input).append_to(&mut args);
        let mut output = run(
            &self.program,
            &args,
            self.timeout,
            self.cancellation.clone(),
            decode,
            |_| {},
        )?;
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
