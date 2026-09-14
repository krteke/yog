use tokio::process::ChildStdout;
use tokio_util::{io::SyncIoBridge, sync::CancellationToken};
mod args;
pub mod pixel_format;
mod streaming;
pub mod types;

use crate::{
    error::{Error, Failure, ProbeError},
    program::Program,
};
use args::{Arg, ArgsExt, Entries};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::BufReader,
    path::{Path, PathBuf},
    time::Duration,
};
use types::{Chapter, Frame, MediaFormat, MediaInfo, MediaStream, Packet};

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
    #[serde(default)]
    pixel_formats: Vec<types::PixelFormat>,
    error: Option<ProbeError>,
}

#[derive(Debug, Clone)]
pub struct Ffprobe {
    inner: Program,
    data_hashes: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PacketFingerprint {
    pub packets: u64,
    pub sha256: [u8; 32],
    pub timing_sha256: [u8; 32],
}

impl Ffprobe {
    pub fn new(path: impl Into<PathBuf>, timeout: Option<Duration>) -> Self {
        Self {
            inner: Program {
                path: path.into(),
                timeout,
                cancellation: CancellationToken::new(),
            },
            data_hashes: false,
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

    pub fn with_data_hashes(mut self, enabled: bool) -> Self {
        self.data_hashes = enabled;
        self
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.inner.timeout
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.inner.cancellation.clone()
    }

    pub async fn probe(&self, input: &Path) -> Result<ProbeResult<MediaInfo>, Error> {
        self.probe_media(
            input,
            [
                Arg::ShowFormat,
                Arg::ShowStreams,
                Arg::ShowChapters,
                Arg::ShowPrograms,
                Arg::ShowPixelFormats,
            ]
            .into_iter()
            .chain(self.data_hashes.then_some(Arg::DataHash)),
        )
        .await
    }

    pub(crate) async fn probe_stream_layout(
        &self,
        input: &Path,
    ) -> Result<ProbeResult<MediaInfo>, Error> {
        self.probe_media(
            input,
            [
                Arg::ShowFormat,
                Arg::ShowStreams,
                Arg::ShowEntries(Entries::StreamLayout),
            ],
        )
        .await
    }

    pub async fn packet_fingerprints(
        &self,
        input: &Path,
        stream_indices: &[usize],
    ) -> Result<ProbeResult<BTreeMap<usize, PacketFingerprint>>, Error> {
        #[derive(Deserialize)]
        struct PacketHash {
            stream_index: usize,
            data_hash: String,
            pts_time: Option<String>,
            duration_time: Option<String>,
        }
        #[derive(Default)]
        struct StreamHashes {
            packets: u64,
            data: Sha256,
            timing: Sha256,
        }

        let mut hashes: BTreeMap<_, _> = stream_indices
            .iter()
            .map(|&index| (index, StreamHashes::default()))
            .collect();
        let select = match stream_indices {
            [index] => Some(Arg::SelectStream(*index)),
            _ => None,
        };

        self.query(
            input,
            [
                Arg::ShowPackets,
                Arg::DataHash,
                Arg::ShowEntries(Entries::PacketHash),
            ]
            .into_iter()
            .chain(select),
            move |reader| {
                let error = streaming::records(reader, "packets", &mut |packet: PacketHash| {
                    let Some(hash) = hashes.get_mut(&packet.stream_index) else {
                        return;
                    };
                    hash.data.update(packet.data_hash.as_bytes());
                    hash.data.update(b"\n");
                    for time in [packet.pts_time, packet.duration_time] {
                        hash.timing.update(time.as_deref().unwrap_or("").as_bytes());
                        hash.timing.update(b"\n");
                    }
                    hash.packets += 1;
                })
                .map_err(Failure::Json)?;
                Ok((
                    hashes
                        .into_iter()
                        .map(|(index, hash)| {
                            (
                                index,
                                PacketFingerprint {
                                    packets: hash.packets,
                                    sha256: hash.data.finalize().into(),
                                    timing_sha256: hash.timing.finalize().into(),
                                },
                            )
                        })
                        .collect(),
                    error,
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
        self.fold_packet_records(
            input,
            stream_index,
            read_intervals,
            Entries::Packet,
            (),
            move |(), packet| consume(packet),
        )
        .await
    }

    pub(crate) async fn fold_packet_sizes<T>(
        &self,
        input: &Path,
        stream_index: Option<usize>,
        read_intervals: &str,
        state: T,
        fold: impl FnMut(&mut T, Packet) + Send + 'static,
    ) -> Result<ProbeResult<T>, Error>
    where
        T: Send + 'static,
    {
        self.fold_packet_records(
            input,
            stream_index,
            read_intervals,
            Entries::PacketSize,
            state,
            fold,
        )
        .await
    }

    async fn fold_packet_records<T>(
        &self,
        input: &Path,
        stream_index: Option<usize>,
        read_intervals: &str,
        entries: Entries,
        mut state: T,
        mut fold: impl FnMut(&mut T, Packet) + Send + 'static,
    ) -> Result<ProbeResult<T>, Error>
    where
        T: Send + 'static,
    {
        let args = [
            Arg::ReadIntervals(read_intervals),
            Arg::ShowPackets,
            Arg::ShowEntries(entries),
        ]
        .into_iter()
        .chain(stream_index.map(Arg::SelectStream));
        self.query(input, args, move |reader| {
            let error =
                streaming::records(reader, "packets", &mut |packet| fold(&mut state, packet))
                    .map_err(Failure::Json)?;
            Ok((state, error))
        })
        .await
    }

    async fn probe_media<'a>(
        &self,
        input: &Path,
        options: impl IntoIterator<Item = Arg<'a>>,
    ) -> Result<ProbeResult<MediaInfo>, Error> {
        self.query(input, options, move |reader| {
            let response: MediaResponse = serde_json::from_reader(reader).map_err(Failure::Json)?;
            Ok((
                MediaInfo {
                    streams: response.streams,
                    chapters: response.chapters,
                    programs: response.programs,
                    format: response.format,
                    pixel_formats: response.pixel_formats,
                },
                response.error,
            ))
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
        args.extend([Arg::ErrorsOnly, Arg::Json, Arg::ShowError]);
        args.extend(options);
        args.add(Arg::Input(input));

        let command = self.inner.build(args);

        let mut output = command
            .run(
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
