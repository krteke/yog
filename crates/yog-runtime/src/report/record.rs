use std::{borrow::Cow, path::Path};

use serde::Serialize;
use yog_core::{
    ffmpeg::{
        encoding::RateControl,
        plan::{TranscodeRequest, VideoAction},
        prediction::{Estimate, Prediction, PredictionOptions, PredictionSample},
        vmaf::VmafOptions,
    },
    ffprobe::types::{MediaInfo, MediaStream},
};

use crate::report::{Record, Status, ToAbsolute, container, percent, transcode::TranscodeResult};

pub trait TaskRecord {
    fn finish(self, status: Status, error: Option<String>) -> Record;
}

#[derive(Debug, Serialize)]
pub struct SkippedRecord {
    input: String,
    reason: String,
}

impl SkippedRecord {
    pub fn new(input: &Path, error: &anyhow::Error) -> Self {
        Self {
            input: input.to_absolute(),
            reason: format!("{error:#}"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SamplingRecord {
    requested_samples: usize,
    sample_seconds: f64,
    measured_samples: usize,
    sampled_seconds: f64,
}

#[derive(Debug, Serialize)]
pub struct QualityRecord {
    frames: usize,
    source_stream_index: usize,
    vmaf: EstimateRecord<f64>,
    ssim: Option<EstimateRecord<f64>>,
    psnr_y_db: Option<EstimateRecord<f64>>,
}

#[derive(Debug, Serialize)]
pub struct SampleRecord {
    start_seconds: f64,
    duration_seconds: f64,
    encode_seconds: f64,
    speed: f64,
    timed_payload_bytes: u64,
    scored_frames: usize,
    vmaf: f64,
    ssim: Option<f64>,
    psnr_y_db: Option<f64>,
}

impl From<&PredictionSample> for SampleRecord {
    fn from(sample: &PredictionSample) -> Self {
        Self {
            start_seconds: sample.start_seconds,
            duration_seconds: sample.duration_seconds,
            encode_seconds: sample.encode_seconds,
            speed: sample.speed,
            timed_payload_bytes: sample.timed_payload_bytes,
            scored_frames: sample.scored_frames,
            vmaf: sample.vmaf,
            ssim: sample.ssim,
            psnr_y_db: sample.psnr_y_db,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EstimateRecord<T> {
    value: T,
    low: T,
    high: T,
}

impl<T: Copy> From<Estimate<T>> for EstimateRecord<T> {
    fn from(estimate: Estimate<T>) -> Self {
        Self {
            value: estimate.value,
            low: estimate.low,
            high: estimate.high,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct VideoRecord {
    action: &'static str,
    encoder: Option<&'static str>,
    rate: Option<RateRecord>,
    preset: Option<Cow<'static, str>>,
    multipass: Option<&'static str>,
}

impl From<&VideoAction> for VideoRecord {
    fn from(value: &VideoAction) -> Self {
        let VideoAction::Encode(encoding) = value else {
            return Self {
                action: "copy",
                encoder: None,
                rate: None,
                preset: None,
                multipass: None,
            };
        };

        Self {
            action: "encode",
            encoder: Some(encoding.name()),
            rate: encoding.rate().map(|rate| match rate {
                RateControl::Quality(value) => RateRecord {
                    kind: "quality",
                    parameter: encoding.quality_parameter(),
                    value: value.into(),
                },
                RateControl::Bitrate(value) => RateRecord {
                    kind: "bitrate",
                    parameter: "b",
                    value: value.get(),
                },
            }),
            preset: encoding.preset(),
            multipass: encoding.multipass(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RateRecord {
    kind: &'static str,
    parameter: &'static str,
    value: u64,
}

#[derive(Debug, Serialize)]
pub struct SourceRecord {
    bytes: Option<u64>,
    duration_seconds: Option<f64>,
    streams: StreamCounts,
    video: Option<VideoStreamRecord>,
    audio: Vec<AudioStreamRecord>,
}

impl SourceRecord {
    fn new(media: &MediaInfo, input: &Path) -> Self {
        let mut streams = StreamCounts::default();
        for stream in &media.streams {
            match stream.codec_type.as_deref() {
                Some("video") if stream.is_regular_video() => streams.video += 1,
                Some("video") => streams.cover += 1,
                Some("audio") => streams.audio += 1,
                Some("subtitle") => streams.subtitle += 1,
                Some("attachment") => streams.attachment += 1,
                _ => streams.other += 1,
            }
        }

        Self {
            bytes: input.metadata().ok().map(|metadata| metadata.len()),
            duration_seconds: media
                .format
                .try_duration()
                .ok()
                .map(|duration| duration.as_secs_f64()),
            streams,
            video: media
                .streams
                .iter()
                .find(|stream| stream.is_regular_video())
                .map(VideoStreamRecord::from),
            audio: media
                .streams
                .iter()
                .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
                .map(AudioStreamRecord::from)
                .collect(),
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct StreamCounts {
    video: usize,
    audio: usize,
    subtitle: usize,
    attachment: usize,
    cover: usize,
    other: usize,
}

#[derive(Debug, Serialize)]
pub struct VideoStreamRecord {
    codec: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    frame_rate: Option<String>,
    bit_rate: Option<String>,
}

impl From<&MediaStream> for VideoStreamRecord {
    fn from(stream: &MediaStream) -> Self {
        Self {
            codec: stream.codec_name.clone(),
            width: stream.width,
            height: stream.height,
            frame_rate: stream.avg_frame_rate.clone(),
            bit_rate: stream.bit_rate.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AudioStreamRecord {
    codec: Option<String>,
    channels: Option<u32>,
    sample_rate: Option<String>,
}

impl From<&MediaStream> for AudioStreamRecord {
    fn from(stream: &MediaStream) -> Self {
        Self {
            codec: stream.codec_name.clone(),
            channels: stream.channels,
            sample_rate: stream.sample_rate.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PredictRecord {
    input: String,
    status: Status,
    container: Option<String>,
    video: VideoRecord,
    decoding: String,
    sampling: SamplingRecord,
    source: Option<SourceRecord>,
    speed: Option<EstimateRecord<f64>>,
    transcode_seconds: Option<EstimateRecord<f64>>,
    output_bytes: Option<EstimateRecord<u64>>,
    size_percent: Option<f64>,
    quality: Option<QualityRecord>,
    samples: Vec<SampleRecord>,
    error: Option<String>,
}

impl PredictRecord {
    pub fn new(request: &TranscodeRequest, options: PredictionOptions) -> Self {
        Self {
            input: request.input.to_absolute(),
            status: Status::Success,
            container: None,
            video: VideoRecord::from(&request.video),
            decoding: request.decoding.to_string(),
            sampling: SamplingRecord {
                requested_samples: options.samples.get(),
                sample_seconds: options.sample_duration.as_secs_f64(),
                measured_samples: 0,
                sampled_seconds: 0.0,
            },
            source: None,
            speed: None,
            transcode_seconds: None,
            output_bytes: None,
            size_percent: None,
            quality: None,
            samples: Vec::new(),
            error: None,
        }
    }

    pub fn fill_source(&mut self, request: &TranscodeRequest, media: &MediaInfo) {
        self.container = container(request, media);
        self.source = Some(SourceRecord::new(media, &request.input));
    }

    pub fn fill_prediction(&mut self, prediction: &Prediction) {
        self.sampling.measured_samples = prediction.samples.len();
        self.sampling.sampled_seconds = prediction.sampled_seconds;
        self.speed = Some(prediction.speed.into());
        self.transcode_seconds = Some(prediction.transcode_seconds.into());
        self.output_bytes = Some(prediction.output_bytes.into());
        self.size_percent = percent(prediction.output_bytes.value, prediction.source_bytes);
        self.quality = Some(QualityRecord {
            frames: prediction.quality.frames,
            source_stream_index: prediction.quality.source_stream_index,
            vmaf: prediction.quality.vmaf.into(),
            ssim: prediction.quality.ssim.map(Into::into),
            psnr_y_db: prediction.quality.psnr_y_db.map(Into::into),
        });
        self.samples = prediction.samples.iter().map(SampleRecord::from).collect();
    }

    pub fn set_rate(&mut self, parameter: &'static str, value: u8) {
        self.video.rate = Some(RateRecord {
            kind: "quality",
            parameter,
            value: value.into(),
        });
    }

    pub fn set_outcome(&mut self, status: Status, error: Option<String>) {
        self.status = status;
        self.error = error;
    }
}

impl TaskRecord for PredictRecord {
    fn finish(mut self, status: Status, error: Option<String>) -> Record {
        self.set_outcome(status, error);
        Record::Predict(self)
    }
}

#[derive(Debug, Serialize)]
pub struct OutputsRecord {
    png: Option<String>,
    svg: Option<String>,
}

impl OutputsRecord {
    fn new(png: Option<&Path>, svg: Option<&Path>) -> Self {
        Self {
            png: png.map(ToAbsolute::to_absolute),
            svg: svg.map(ToAbsolute::to_absolute),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EmulateRecord {
    #[serde(flatten)]
    predict: PredictRecord,
    outputs: OutputsRecord,
}

impl EmulateRecord {
    pub fn new(
        request: &TranscodeRequest,
        quality: u8,
        png: Option<&Path>,
        svg: Option<&Path>,
        prediction_options: PredictionOptions,
    ) -> Self {
        let mut predict = PredictRecord::new(request, prediction_options);
        if let VideoAction::Encode(encoding) = &request.video {
            predict.set_rate(encoding.quality_parameter(), quality);
        }

        Self {
            predict,
            outputs: OutputsRecord::new(png, svg),
        }
    }

    pub fn fill_source(&mut self, request: &TranscodeRequest, media: &MediaInfo) {
        self.predict.fill_source(request, media);
    }

    pub fn fill_prediction(&mut self, prediction: &Prediction) {
        self.predict.fill_prediction(prediction);
    }
}

impl TaskRecord for EmulateRecord {
    fn finish(mut self, status: Status, error: Option<String>) -> Record {
        self.predict.set_outcome(status, error);
        Record::Emulate(self)
    }
}

#[derive(Debug, Serialize)]
pub struct TranscodeRecord {
    input: String,
    output: String,
    status: Status,
    container: Option<String>,
    video: VideoRecord,
    decoding: String,
    source: Option<SourceRecord>,
    result: Option<TranscodeResult>,
    verify: Option<VerifyRecord>,
    vmaf: Option<VmafRecord>,
    error: Option<String>,
}

impl TranscodeRecord {
    pub fn new(request: &TranscodeRequest) -> Self {
        Self {
            input: request.input.to_absolute(),
            output: request.output.to_absolute(),
            status: Status::Success,
            container: None,
            video: VideoRecord::from(&request.video),
            decoding: request.decoding.to_string(),
            source: None,
            result: None,
            verify: None,
            vmaf: None,
            error: None,
        }
    }

    pub fn fill_source(&mut self, request: &TranscodeRequest, media: &MediaInfo) {
        self.container = container(request, media);
        self.source = Some(SourceRecord::new(media, &request.input));
    }

    pub fn fill_result(&mut self, mut result: TranscodeResult) {
        result.size_percent = self
            .source
            .as_ref()
            .and_then(|source| source.bytes)
            .zip(result.bytes)
            .and_then(|(total, bytes)| percent(bytes, total));
        self.result = Some(result);
    }

    pub fn fill_verify(&mut self, verify: VerifyRecord) {
        self.verify = Some(verify);
    }

    pub fn fill_vmaf(&mut self, vmaf: VmafRecord) {
        self.vmaf = Some(vmaf);
    }
}

impl TaskRecord for TranscodeRecord {
    fn finish(mut self, status: Status, error: Option<String>) -> Record {
        self.status = status;
        self.error = error;
        Record::Transcode(self)
    }
}

#[derive(Debug, Serialize)]
pub struct VerifyRecord {
    outcome: VerifyOutcome,
    warnings: Vec<String>,
}

impl VerifyRecord {
    pub fn new(outcome: VerifyOutcome, warnings: Vec<String>) -> Self {
        Self { outcome, warnings }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyOutcome {
    Complete,
    MissingAudioAfterSeek,
    Incomplete,
}

#[derive(Debug, Serialize)]
pub struct VmafRecord {
    n_subsample: Option<u32>,
    score: Option<f64>,
    error: Option<String>,
}

impl VmafRecord {
    pub fn requested(options: VmafOptions) -> Self {
        Self {
            n_subsample: options.n_subsample.map(|value| value.get()),
            score: None,
            error: None,
        }
    }

    pub fn scored(mut self, score: f64) -> Self {
        self.score = Some(score);
        self
    }

    pub fn failed(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }
}
