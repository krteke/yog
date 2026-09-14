use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    io::BufReader,
    num::NonZeroUsize,
    path::Path,
    time::{Duration, Instant},
};

use super::{
    Ffmpeg,
    args::Arg,
    plan::{AttachmentInput, TranscodeRequest, VideoAction},
    vmaf::VmafLog,
};
use crate::{
    error::{Failure, PredictionError, ProbeValueError},
    ffprobe::{
        Ffprobe,
        types::{MediaInfo, MediaStream},
    },
};

#[derive(Debug, Clone, Copy)]
pub struct PredictionOptions {
    pub samples: NonZeroUsize,
    pub sample_duration: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct Estimate<T> {
    pub value: T,
    pub low: T,
    pub high: T,
}

#[derive(Debug)]
pub struct Prediction {
    pub source_bytes: u64,
    pub source_duration_seconds: f64,
    pub sampled_seconds: f64,
    pub speed: Estimate<f64>,
    pub transcode_seconds: Estimate<f64>,
    pub output_bytes: Estimate<u64>,
    pub quality: QualityPrediction,
    pub samples: Vec<PredictionSample>,
}

#[derive(Debug)]
pub struct QualityPrediction {
    pub frames: usize,
    pub vmaf: Estimate<f64>,
    pub ssim: Option<Estimate<f64>>,
    pub psnr_y_db: Option<Estimate<f64>>,
    pub source_stream_index: usize,
}

#[derive(Debug)]
pub struct PredictionSample {
    pub start_seconds: f64,
    pub duration_seconds: f64,
    pub encode_seconds: f64,
    pub speed: f64,
    pub timed_payload_bytes: u64,
    pub vmaf: f64,
    pub ssim: Option<f64>,
    pub psnr_y_db: Option<f64>,
    pub scored_frames: usize,
}

#[derive(Debug, Clone, Copy)]
struct SampleWindow {
    start: f64,
    duration: f64,
}

#[derive(Debug)]
struct SampleLayout {
    video_stream_index: usize,
    untimed_streams: HashSet<usize>,
    fixed_extradata_bytes: u64,
}

impl SampleLayout {
    fn from_media(media: &MediaInfo) -> Result<Self, PredictionError> {
        let video_stream_index = media
            .streams
            .iter()
            .find(|stream| stream.is_regular_video())
            .map(|stream| stream.index)
            .ok_or(PredictionError::NoSampleVideo)?;
        let mut untimed_streams = HashSet::new();
        let mut fixed_extradata_bytes = 0_u64;
        for stream in &media.streams {
            let attachment = stream.codec_type.as_deref() == Some("attachment");
            let attached_picture =
                stream.codec_type.as_deref() == Some("video") && !stream.is_regular_video();
            if !attachment && !attached_picture {
                continue;
            }

            untimed_streams.insert(stream.index);
            let size = if attachment {
                stream
                    .extradata_size
                    .ok_or(PredictionError::MissingAttachmentSize {
                        stream_index: stream.index,
                    })?
            } else {
                stream.extradata_size.unwrap_or(0)
            };
            fixed_extradata_bytes = fixed_extradata_bytes
                .checked_add(size)
                .ok_or(PredictionError::ByteCountOverflow)?;
        }
        Ok(Self {
            video_stream_index,
            untimed_streams,
            fixed_extradata_bytes,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct SampleQuality {
    vmaf: f64,
    ssim: Option<f64>,
    psnr_y_db: Option<f64>,
    scored_frames: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct PacketBytes {
    timed: u64,
    untimed: u64,
}

#[derive(Debug, Clone, Copy)]
struct SizeSample {
    scalable_bytes: u64,
    fixed_bytes: u64,
}

#[derive(Debug)]
struct MeasuredSample {
    prediction: PredictionSample,
    size: SizeSample,
}

impl Ffmpeg {
    pub async fn predict(
        &self,
        probe: &Ffprobe,
        request: &TranscodeRequest,
        media: &MediaInfo,
        options: PredictionOptions,
        mut on_stderr: impl FnMut(&[u8]) + Send,
    ) -> Result<Prediction, PredictionError> {
        if !matches!(request.video, VideoAction::Encode(_)) {
            return Err(PredictionError::RequiresEncoding);
        }

        let source_stream = media
            .streams
            .iter()
            .find(|stream| stream.is_regular_video())
            .ok_or(PredictionError::NoVideo)?;
        if !matches!(
            (source_stream.width, source_stream.height),
            (Some(width), Some(height)) if width > 0 && height > 0
        ) {
            return Err(PredictionError::MissingVideoDimensions {
                stream_index: source_stream.index,
            });
        }

        let duration = media.format.try_duration()?.as_secs_f64();
        let video_span = video_span(media, source_stream, duration)?;

        let sample_duration = options.sample_duration.as_secs_f64();

        let windows = sample_windows(video_span, options.samples, sample_duration)?;
        let source_bytes = fs::metadata(&request.input)?.len();
        let base = request.plan(media, self).await?;
        let fixed_attachment_bytes = base
            .attachments
            .iter()
            .filter(|attachment| attachment.input == AttachmentInput::AttachmentStream)
            .try_fold(0u64, |total, attachment| {
                let size = media
                    .streams
                    .iter()
                    .find(|stream| stream.index == attachment.input_index)
                    .and_then(|stream| stream.extradata_size)
                    .ok_or(PredictionError::MissingAttachmentSize {
                        stream_index: attachment.input_index,
                    })?;
                total
                    .checked_add(size)
                    .ok_or(PredictionError::ByteCountOverflow)
            })?;

        let mut samples = Vec::with_capacity(windows.len());
        for (index, window) in windows.iter().copied().enumerate() {
            let directory = tempfile::Builder::new()
                .prefix("yog-prediction-sample-")
                .tempdir()?;
            let output = directory.path().join("output");
            let start = format_seconds(window.start);
            let length = format_seconds(window.duration);
            let plan = base.sample(&start, &length, output.clone(), index == 0);
            let command = self.build(&plan)?;

            let started = Instant::now();
            let mut reported_speed = None;
            command
                .run(
                    |progress| {
                        if let (Some(speed), Some(microseconds)) =
                            (progress.speed, progress.out_time_us)
                            && speed > 0.0
                            && microseconds > 0
                        {
                            reported_speed = Some((speed, microseconds as f64 / 1_000_000.0));
                        }
                    },
                    &mut on_stderr,
                )
                .await?;
            let encode_seconds = started.elapsed().as_secs_f64();

            let sample_media = probe.probe_sample(&output).await?.output;
            let speed = reported_speed
                .map_or(window.duration / encode_seconds, |(speed, seconds)| {
                    window.duration * speed / seconds
                });
            let SampleLayout {
                video_stream_index,
                untimed_streams,
                fixed_extradata_bytes,
            } = SampleLayout::from_media(&sample_media)?;
            let packet_bytes = packet_bytes(probe, &output, untimed_streams).await?;
            if packet_bytes.timed == 0 {
                return Err(PredictionError::NoTimedPackets { sample: index + 1 });
            }
            let file_bytes = fs::metadata(&output)?.len();
            let fixed_payload_bytes = packet_bytes
                .untimed
                .checked_add(fixed_extradata_bytes)
                .ok_or(PredictionError::ByteCountOverflow)?;
            let payload_bytes = packet_bytes
                .timed
                .checked_add(fixed_payload_bytes)
                .ok_or(PredictionError::ByteCountOverflow)?;
            let mux_bytes = file_bytes
                .checked_sub(payload_bytes)
                .ok_or(PredictionError::PacketPayloadExceedsFile { sample: index + 1 })?;
            let size = SizeSample {
                scalable_bytes: packet_bytes.timed + mux_bytes,
                fixed_bytes: fixed_payload_bytes,
            };

            let quality = self
                .score_sample(
                    &request.input,
                    source_stream,
                    &output,
                    video_stream_index,
                    &start,
                    &mut on_stderr,
                )
                .await?;
            samples.push(MeasuredSample {
                prediction: PredictionSample {
                    start_seconds: window.start,
                    duration_seconds: window.duration,
                    encode_seconds,
                    speed,
                    timed_payload_bytes: packet_bytes.timed,
                    vmaf: quality.vmaf,
                    ssim: quality.ssim,
                    psnr_y_db: quality.psnr_y_db,
                    scored_frames: quality.scored_frames,
                },
                size,
            });
        }

        summarize(
            source_bytes,
            duration,
            source_stream.index,
            fixed_attachment_bytes,
            samples,
        )
    }

    async fn score_sample(
        &self,
        source: &Path,
        source_stream: &MediaStream,
        candidate: &Path,
        candidate_stream_index: usize,
        start: &str,
        on_stderr: impl FnMut(&[u8]),
    ) -> Result<SampleQuality, PredictionError> {
        let source_width = source_stream.width.expect("source dimensions validated");
        let source_height = source_stream.height.expect("source dimensions validated");
        let metrics_path = candidate.with_extension("vmaf.json");
        let filter = format!(
            "[0:{}]crop=w={}:h={}:x=0:y=0:exact=1,setpts=PTS-STARTPTS[dist];\
             [1:{}]setpts=PTS-STARTPTS[ref];\
             [dist][ref]libvmaf=feature=name=psnr|name=float_ssim:log_fmt=json:log_path={}:shortest=1[out]",
            candidate_stream_index,
            source_width,
            source_height,
            source_stream.index,
            escape_filter_path(&metrics_path)
        );
        let mut args: Vec<OsString> = Vec::new();
        args.extend([
            Arg::HideBanner,
            Arg::NoStdin,
            Arg::LogLevel("error"),
            Arg::Input(candidate),
            Arg::Seek(start),
            Arg::Input(source),
            Arg::FilterComplex(&filter),
            Arg::MapLabel("[out]"),
            Arg::Format("null"),
            Arg::Stdout,
        ]);
        let command = self.inner.build(args);
        let output = command.run(|_| async { Ok(()) }, on_stderr).await?;
        if !output.status.success() {
            return Err(output.failure(Failure::Exit).into());
        }

        let log: VmafLog = serde_json::from_reader(BufReader::new(fs::File::open(metrics_path)?))?;
        if log.frames.is_empty() {
            return Err(PredictionError::NoScoredFrames);
        }
        let vmaf =
            metric_sample(&log, |metrics| metrics.vmaf).ok_or(PredictionError::NoVmafScore)?;
        Ok(SampleQuality {
            vmaf,
            ssim: metric_sample(&log, |metrics| metrics.float_ssim),
            psnr_y_db: metric_sample(&log, |metrics| metrics.psnr_y),
            scored_frames: log.frames.len(),
        })
    }
}

fn video_span(
    media: &MediaInfo,
    stream: &MediaStream,
    source_duration: f64,
) -> Result<SampleWindow, PredictionError> {
    let format_start = media.format.try_start_time()?.unwrap_or(0.0);
    let format_end = format_start + source_duration;
    let stream_start = stream.try_start_time()?.unwrap_or(format_start);
    let start = stream_start.max(format_start);
    let end = stream
        .try_duration()?
        .map(|duration| stream_start + duration.as_secs_f64())
        .unwrap_or(format_end)
        .min(format_end);
    let duration = end - start;
    if !format_end.is_finite() || !duration.is_finite() || duration <= 0.0 {
        return Err(PredictionError::InvalidVideoSpan {
            stream_index: stream.index,
        });
    }
    Ok(SampleWindow {
        start: start - format_start,
        duration,
    })
}

fn sample_windows(
    span: SampleWindow,
    requested: NonZeroUsize,
    requested_seconds: f64,
) -> Result<Vec<SampleWindow>, PredictionError> {
    let sample_duration = requested_seconds.min(span.duration);
    let count = requested
        .get()
        .min((span.duration / sample_duration).floor() as usize);
    debug_assert!(count > 0);

    Ok((0..count)
        .map(|index| {
            let center = span.duration * (index as f64 + 0.5) / count as f64;
            SampleWindow {
                start: span.start
                    + (center - sample_duration / 2.0).clamp(0.0, span.duration - sample_duration),
                duration: sample_duration,
            }
        })
        .collect())
}

async fn packet_bytes(
    probe: &Ffprobe,
    path: &Path,
    untimed_streams: HashSet<usize>,
) -> Result<PacketBytes, PredictionError> {
    #[derive(Debug)]
    enum PacketSumError {
        Value(ProbeValueError),
        Overflow,
    }
    #[derive(Debug, Default)]
    struct PacketState {
        bytes: PacketBytes,
        error: Option<PacketSumError>,
    }

    let state = probe
        .fold_packet_sizes(
            path,
            None,
            "%",
            PacketState::default(),
            move |state, packet| {
                if state.error.is_some() {
                    return;
                }
                let size = match packet.try_size() {
                    Ok(size) => size,
                    Err(error) => {
                        state.error = Some(PacketSumError::Value(error));
                        return;
                    }
                };
                let untimed = untimed_streams.contains(&packet.stream_index);
                let current = if untimed {
                    state.bytes.untimed
                } else {
                    state.bytes.timed
                };
                let Some(total) = current.checked_add(size) else {
                    state.error = Some(PacketSumError::Overflow);
                    return;
                };
                if untimed {
                    state.bytes.untimed = total;
                } else {
                    state.bytes.timed = total;
                }
            },
        )
        .await?
        .output;
    match state.error {
        Some(PacketSumError::Value(error)) => Err(error.into()),
        Some(PacketSumError::Overflow) => Err(PredictionError::ByteCountOverflow),
        None => Ok(state.bytes),
    }
}

fn metric_sample(log: &VmafLog, get: impl Fn(&super::vmaf::Metrics) -> Option<f64>) -> Option<f64> {
    if log.frames.is_empty() {
        return None;
    }
    let sum = log
        .frames
        .iter()
        .try_fold(0.0, |sum, frame| Some(sum + get(&frame.metrics)?))?;
    Some(sum / log.frames.len() as f64)
}

fn summarize(
    source_bytes: u64,
    source_duration: f64,
    source_stream_index: usize,
    attachment_bytes: u64,
    samples: Vec<MeasuredSample>,
) -> Result<Prediction, PredictionError> {
    let sampled_seconds = samples
        .iter()
        .map(|sample| sample.prediction.duration_seconds)
        .sum::<f64>();
    let speed = estimate(
        samples.iter().map(|sample| sample.prediction.speed),
        aggregate_speed(
            samples
                .iter()
                .map(|sample| (sample.prediction.duration_seconds, sample.prediction.speed)),
        ),
    );
    let transcode_seconds = Estimate {
        value: source_duration / speed.value,
        low: source_duration / speed.high,
        high: source_duration / speed.low,
    };
    let output_bytes = estimate_output_bytes(&samples, source_duration, attachment_bytes)?;
    let (vmaf, frames) = aggregate_metric(&samples, |sample| Some(sample.vmaf))
        .expect("every measured sample has a VMAF score");
    let ssim = aggregate_metric(&samples, |sample| sample.ssim).map(|(estimate, _)| estimate);
    let psnr_y_db =
        aggregate_metric(&samples, |sample| sample.psnr_y_db).map(|(estimate, _)| estimate);

    Ok(Prediction {
        source_bytes,
        source_duration_seconds: source_duration,
        sampled_seconds,
        speed,
        transcode_seconds,
        output_bytes,
        quality: QualityPrediction {
            frames,
            vmaf,
            ssim,
            psnr_y_db,
            source_stream_index,
        },
        samples: samples
            .into_iter()
            .map(|sample| sample.prediction)
            .collect(),
    })
}

fn aggregate_metric(
    samples: &[MeasuredSample],
    get: impl Fn(&PredictionSample) -> Option<f64>,
) -> Option<(Estimate<f64>, usize)> {
    let mut samples = samples.iter();
    let first = &samples.next()?.prediction;
    let first_value = get(first)?;
    let mut low = first_value;
    let mut high = first_value;
    let mut weighted_sum = first_value * first.scored_frames as f64;
    let mut frames = first.scored_frames;
    for sample in samples {
        let value = get(&sample.prediction)?;
        low = low.min(value);
        high = high.max(value);
        weighted_sum += value * sample.prediction.scored_frames as f64;
        frames += sample.prediction.scored_frames;
    }
    Some((
        Estimate {
            value: weighted_sum / frames as f64,
            low,
            high,
        },
        frames,
    ))
}

fn estimate(values: impl IntoIterator<Item = f64>, value: f64) -> Estimate<f64> {
    let mut values = values.into_iter();
    let first = values.next().expect("samples exist");
    let (low, high) = values.fold((first, first), |(low, high), sample| {
        (low.min(sample), high.max(sample))
    });
    Estimate { value, low, high }
}

fn aggregate_speed(samples: impl IntoIterator<Item = (f64, f64)>) -> f64 {
    let (media_seconds, wall_seconds) = samples
        .into_iter()
        .fold((0.0, 0.0), |(media, wall), (duration, speed)| {
            (media + duration, wall + duration / speed)
        });
    media_seconds / wall_seconds
}

fn estimate_output_bytes(
    samples: &[MeasuredSample],
    duration: f64,
    attachment_bytes: u64,
) -> Result<Estimate<u64>, PredictionError> {
    let (total_bytes, total_duration) = samples.iter().fold((0.0, 0.0), |total, sample| {
        (
            total.0 + sample.size.scalable_bytes as f64,
            total.1 + sample.prediction.duration_seconds,
        )
    });
    let average_rate = total_bytes / total_duration;
    let rate = estimate(
        samples
            .iter()
            .map(|sample| sample.size.scalable_bytes as f64 / sample.prediction.duration_seconds),
        average_rate,
    );
    let fixed_bytes = attachment_bytes as f64
        + samples
            .iter()
            .map(|sample| sample.size.fixed_bytes as f64)
            .reduce(f64::max)
            .expect("samples exist");
    Ok(Estimate {
        value: predicted_bytes(rate.value, duration, fixed_bytes)?,
        low: predicted_bytes(rate.low, duration, fixed_bytes)?,
        high: predicted_bytes(rate.high, duration, fixed_bytes)?,
    })
}

fn format_seconds(seconds: f64) -> String {
    format!("{seconds:.9}")
}

fn predicted_bytes(
    bytes_per_second: f64,
    duration: f64,
    fixed_bytes: f64,
) -> Result<u64, PredictionError> {
    let value = bytes_per_second * duration + fixed_bytes;
    if !value.is_finite() || value < 0.0 || value >= u64::MAX as f64 {
        return Err(PredictionError::OutputSizeOverflow);
    }
    Ok(value.round() as u64)
}

fn escape_filter_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let escaped = normalized.replace('\'', r"'\''").replace(':', r"\:");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn non_zero(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).unwrap()
    }

    fn span(start: f64, duration: f64) -> SampleWindow {
        SampleWindow { start, duration }
    }

    fn measured_sample(
        prediction: PredictionSample,
        scalable_bytes: u64,
        fixed_bytes: u64,
    ) -> MeasuredSample {
        MeasuredSample {
            prediction,
            size: SizeSample {
                scalable_bytes,
                fixed_bytes,
            },
        }
    }

    #[test]
    fn sample_windows_are_stratified_bounded_and_capacity_limited() {
        let windows = sample_windows(span(0.0, 100.0), non_zero(5), 4.0).unwrap();
        assert_eq!(windows.len(), 5);
        assert_eq!(windows[0].start, 8.0);
        assert_eq!(windows[2].start, 48.0);
        assert_eq!(windows[4].start, 88.0);
        assert!(
            windows
                .windows(2)
                .all(|pair| { pair[0].start + pair[0].duration <= pair[1].start })
        );

        let constrained = sample_windows(span(0.0, 9.0), non_zero(5), 2.0).unwrap();
        assert_eq!(constrained.len(), 4);
        assert!(
            constrained
                .iter()
                .all(|window| { window.start >= 0.0 && window.start + window.duration <= 9.0 })
        );
        assert!(
            constrained
                .windows(2)
                .all(|pair| pair[0].start + pair[0].duration <= pair[1].start)
        );

        let short = sample_windows(span(0.0, 1.5), non_zero(5), 2.0).unwrap();
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].start, 0.0);
        assert_eq!(short[0].duration, 1.5);
    }

    #[test]
    fn video_span_uses_stream_timing_relative_to_the_format() {
        let mut media: MediaInfo = serde_json::from_str(
            r#"{
                "format":{"start_time":"10","duration":"100"},
                "streams":[{"index":3,"codec_type":"video","start_time":"20","duration":"30"}]
            }"#,
        )
        .unwrap();

        let span = video_span(&media, &media.streams[0], 100.0).unwrap();
        assert_eq!(span.start, 10.0);
        assert_eq!(span.duration, 30.0);
        let windows = sample_windows(span, non_zero(3), 10.0).unwrap();
        assert_eq!(
            windows
                .iter()
                .map(|window| window.start)
                .collect::<Vec<_>>(),
            [10.0, 20.0, 30.0]
        );

        media.streams[0].start_time = Some("200".to_owned());
        assert!(matches!(
            video_span(&media, &media.streams[0], 100.0),
            Err(PredictionError::InvalidVideoSpan { stream_index: 3 })
        ));
    }

    #[test]
    fn sample_layout_counts_attachment_extradata_as_fixed_payload() {
        let mut media: MediaInfo = serde_json::from_str(
            r#"{"streams":[
                {"index":2,"codec_type":"video","pix_fmt":"yuv420p"},
                {"index":5,"codec_type":"attachment","extradata_size":100},
                {"index":7,"codec_type":"video","disposition":{"attached_pic":1},"extradata_size":3},
                {"index":8,"codec_type":"audio","extradata_size":999}
            ]}"#,
        )
        .unwrap();

        let layout = SampleLayout::from_media(&media).unwrap();
        assert_eq!(layout.video_stream_index, 2);
        assert_eq!(layout.untimed_streams, HashSet::from([5, 7]));
        assert_eq!(layout.fixed_extradata_bytes, 103);

        media.streams[1].extradata_size = None;
        assert!(matches!(
            SampleLayout::from_media(&media),
            Err(PredictionError::MissingAttachmentSize { stream_index: 5 })
        ));
    }

    #[test]
    fn summary_uses_matching_timelines_and_separates_fixed_bytes() {
        let samples = vec![
            measured_sample(
                PredictionSample {
                    start_seconds: 0.0,
                    duration_seconds: 1.0,
                    encode_seconds: 1.0,
                    speed: 1.0,
                    timed_payload_bytes: 120,
                    vmaf: 80.0,
                    ssim: Some(0.8),
                    psnr_y_db: Some(40.0),
                    scored_frames: 1,
                },
                120,
                50,
            ),
            measured_sample(
                PredictionSample {
                    start_seconds: 0.0,
                    duration_seconds: 3.0,
                    encode_seconds: 0.75,
                    speed: 4.0,
                    timed_payload_bytes: 540,
                    vmaf: 100.0,
                    ssim: Some(1.0),
                    psnr_y_db: None,
                    scored_frames: 3,
                },
                540,
                0,
            ),
        ];

        let prediction = summarize(1_000, 10.0, 7, 25, samples).unwrap();

        assert_eq!(prediction.source_bytes, 1_000);
        assert_eq!(prediction.sampled_seconds, 4.0);
        assert_eq!(prediction.speed.value, 16.0 / 7.0);
        assert_eq!(prediction.speed.low, 1.0);
        assert_eq!(prediction.speed.high, 4.0);
        assert_eq!(prediction.transcode_seconds.value, 4.375);
        assert_eq!(prediction.transcode_seconds.low, 2.5);
        assert_eq!(prediction.transcode_seconds.high, 10.0);
        assert_eq!(prediction.output_bytes.value, 1_725);
        assert_eq!(prediction.output_bytes.low, 1_275);
        assert_eq!(prediction.output_bytes.high, 1_875);
        assert_eq!(prediction.quality.frames, 4);
        assert_eq!(prediction.quality.vmaf.value, 95.0);
        assert_eq!(prediction.quality.vmaf.low, 80.0);
        assert_eq!(prediction.quality.vmaf.high, 100.0);
        assert_eq!(prediction.quality.ssim.unwrap().value, 0.95);
        assert!(prediction.quality.psnr_y_db.is_none());
        assert_eq!(prediction.quality.source_stream_index, 7);
        assert_eq!(prediction.samples.len(), 2);
    }

    #[test]
    fn optional_metric_is_omitted_when_any_scored_frame_lacks_it() {
        let log: VmafLog = serde_json::from_str(
            r#"{"frames":[
                {"frameNum":0,"metrics":{"vmaf":80.0,"float_ssim":0.8}},
                {"frameNum":1,"metrics":{"vmaf":100.0}}
            ]}"#,
        )
        .unwrap();

        let vmaf = metric_sample(&log, |metrics| metrics.vmaf).unwrap();
        assert_eq!(vmaf, 90.0);
        assert!(metric_sample(&log, |metrics| metrics.float_ssim).is_none());
    }

    #[test]
    fn timestamp_precision_and_output_size_bounds_are_preserved() {
        assert_eq!(format_seconds(0.000_001), "0.000001000");
        assert_eq!(predicted_bytes(1.25, 4.0, 2.0).unwrap(), 7);
        for value in [f64::NAN, f64::INFINITY, -1.0, u64::MAX as f64] {
            assert!(matches!(
                predicted_bytes(0.0, 0.0, value),
                Err(PredictionError::OutputSizeOverflow)
            ));
        }
    }
}
