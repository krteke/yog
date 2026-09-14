use super::{
    Ffmpeg,
    args::{Arg, ArgsExt},
};
use crate::{
    error::{Failure, VmafError},
    ffprobe::{Ffprobe, types::MediaInfo},
};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, fs, io::BufReader, num::NonZeroU32, path::Path};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VmafOptions {
    pub n_subsample: Option<NonZeroU32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VmafScore {
    pub value: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VmafLog {
    pub frames: Vec<Frame>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Frame {
    #[serde(rename = "frameNum")]
    pub frame_num: u64,
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Metrics {
    #[serde(default, deserialize_with = "metric")]
    pub vmaf: Option<f64>,
    #[serde(default, deserialize_with = "metric")]
    pub float_ssim: Option<f64>,
    #[serde(default, deserialize_with = "metric")]
    pub psnr_y: Option<f64>,
}

#[derive(Debug)]
pub(super) struct VmafSample {
    pub vmaf: f64,
    pub ssim: Option<f64>,
    pub psnr_y_db: Option<f64>,
    pub scored_frames: usize,
}

pub(super) struct VmafSampleInput<'a> {
    pub reference: &'a Path,
    pub reference_stream_index: usize,
    pub reference_dimensions: (u32, u32),
    pub distorted: &'a Path,
    pub distorted_stream_index: usize,
    pub reference_start: &'a str,
}

#[derive(Default, Deserialize)]
struct VmafSummaryLog {
    #[serde(default)]
    pooled_metrics: PooledMetrics,
}

#[derive(Default, Deserialize)]
struct PooledMetrics {
    #[serde(default)]
    vmaf: Option<PooledMetric>,
}

#[derive(Deserialize)]
struct PooledMetric {
    #[serde(default, deserialize_with = "metric")]
    mean: Option<f64>,
}

struct VmafComparison<'a> {
    distorted: &'a Path,
    distorted_stream_index: usize,
    reference: &'a Path,
    reference_stream_index: usize,
    reference_dimensions: (u32, u32),
    reference_start: Option<&'a str>,
    n_subsample: Option<NonZeroU32>,
    auxiliary_metrics: bool,
}

fn metric<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Option<f64>, D::Error> {
    Ok(serde_json::Value::deserialize(decoder)?.as_f64())
}

impl Ffmpeg {
    pub async fn vmaf(
        &self,
        probe: &Ffprobe,
        reference: &Path,
        reference_media: &MediaInfo,
        distorted: &Path,
        options: VmafOptions,
        on_stderr: impl FnMut(&[u8]),
    ) -> Result<VmafScore, VmafError> {
        let reference_stream = reference_media
            .streams
            .iter()
            .find(|stream| stream.is_regular_video())
            .ok_or(VmafError::NoReferenceVideo)?;
        let dimensions = match (reference_stream.width, reference_stream.height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => (width, height),
            _ => {
                return Err(VmafError::MissingReferenceDimensions {
                    stream_index: reference_stream.index,
                });
            }
        };
        let distorted_media = probe.probe_stream_layout(distorted).await?.output;
        let distorted_stream = distorted_media
            .streams
            .iter()
            .find(|stream| stream.is_regular_video())
            .ok_or(VmafError::NoDistortedVideo)?;

        let value = self
            .run_vmaf(
                VmafComparison {
                    distorted,
                    distorted_stream_index: distorted_stream.index,
                    reference,
                    reference_stream_index: reference_stream.index,
                    reference_dimensions: dimensions,
                    reference_start: None,
                    n_subsample: options.n_subsample,
                    auxiliary_metrics: false,
                },
                |path| {
                    let log: VmafSummaryLog =
                        serde_json::from_reader(BufReader::new(fs::File::open(path)?))?;
                    log.pooled_metrics
                        .vmaf
                        .and_then(|metric| metric.mean)
                        .filter(|score| score.is_finite())
                        .ok_or(VmafError::NoVmafScore)
                },
                on_stderr,
            )
            .await?;

        Ok(VmafScore { value })
    }

    pub(super) async fn vmaf_sample(
        &self,
        input: VmafSampleInput<'_>,
        on_stderr: impl FnMut(&[u8]),
    ) -> Result<VmafSample, VmafError> {
        let VmafSampleInput {
            reference,
            reference_stream_index,
            reference_dimensions,
            distorted,
            distorted_stream_index,
            reference_start,
        } = input;
        self.run_vmaf(
            VmafComparison {
                distorted,
                distorted_stream_index,
                reference,
                reference_stream_index,
                reference_dimensions,
                reference_start: Some(reference_start),
                n_subsample: None,
                auxiliary_metrics: true,
            },
            |path| {
                let log: VmafLog = serde_json::from_reader(BufReader::new(fs::File::open(path)?))?;
                if log.frames.is_empty() {
                    return Err(VmafError::NoScoredFrames);
                }
                let vmaf =
                    metric_sample(&log, |metrics| metrics.vmaf).ok_or(VmafError::NoVmafScore)?;
                Ok(VmafSample {
                    vmaf,
                    ssim: metric_sample(&log, |metrics| metrics.float_ssim),
                    psnr_y_db: metric_sample(&log, |metrics| metrics.psnr_y),
                    scored_frames: log.frames.len(),
                })
            },
            on_stderr,
        )
        .await
    }

    async fn run_vmaf<T>(
        &self,
        comparison: VmafComparison<'_>,
        parse: impl FnOnce(&Path) -> Result<T, VmafError>,
        on_stderr: impl FnMut(&[u8]),
    ) -> Result<T, VmafError> {
        let VmafComparison {
            distorted,
            distorted_stream_index,
            reference,
            reference_stream_index,
            reference_dimensions: (reference_width, reference_height),
            reference_start,
            n_subsample,
            auxiliary_metrics,
        } = comparison;
        let metrics_dir = tempfile::Builder::new().prefix("yog-vmaf-").tempdir()?;
        let metrics_path = metrics_dir.path().join("metrics.json");
        let features = if auxiliary_metrics {
            "feature=name=psnr|name=float_ssim:"
        } else {
            ""
        };
        let subsample = n_subsample
            .map(|value| format!(":n_subsample={value}"))
            .unwrap_or_default();
        let filter = format!(
            "[0:{distorted_stream_index}]crop=w={reference_width}:h={reference_height}:x=0:y=0:exact=1,setpts=PTS-STARTPTS[dist];\
             [1:{reference_stream_index}]setpts=PTS-STARTPTS[ref];\
             [dist][ref]libvmaf={features}log_fmt=json:log_path={}:shortest=1{subsample}[out]",
            escape_filter_path(&metrics_path),
        );
        let mut args: Vec<OsString> = Vec::new();
        args.extend([
            Arg::HideBanner,
            Arg::NoStdin,
            Arg::LogLevel("error"),
            Arg::Input(distorted),
        ]);
        if let Some(start) = reference_start {
            args.add(Arg::Seek(start));
        }
        args.extend([
            Arg::Input(reference),
            Arg::FilterComplex(&filter),
            Arg::MapLabel("[out]"),
            Arg::Format("null"),
            Arg::Stdout,
        ]);

        let output = self
            .inner
            .build(args)
            .run(|_| async { Ok(()) }, on_stderr)
            .await?;
        if !output.status.success() {
            return Err(output.failure(Failure::Exit).into());
        }
        parse(&metrics_path)
    }
}

fn metric_sample(log: &VmafLog, get: impl Fn(&Metrics) -> Option<f64>) -> Option<f64> {
    if log.frames.is_empty() {
        return None;
    }
    let sum = log.frames.iter().try_fold(0.0, |sum, frame| {
        let value = get(&frame.metrics)?;
        if !value.is_finite() {
            return None;
        }
        Some(sum + value)
    })?;
    Some(sum / log.frames.len() as f64)
}

fn escape_filter_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let escaped = normalized.replace('\'', r"'\''").replace(':', r"\:");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_missing_and_non_numeric_metrics_remain_distinct() {
        let log: VmafLog = serde_json::from_str(
            r#"{"frames":[
            {"frameNum":0,"metrics":{"vmaf":98.5,"psnr_y":"inf"}},
            {"frameNum":1,"metrics":{"vmaf":0,"float_ssim":null}}
        ]}"#,
        )
        .unwrap();
        assert_eq!(log.frames[0].metrics.vmaf, Some(98.5));
        assert_eq!(log.frames[0].metrics.psnr_y, None);
        assert_eq!(log.frames[0].metrics.float_ssim, None);
        assert_eq!(log.frames[1].metrics.vmaf, Some(0.0));
        assert_eq!(log.frames[1].metrics.float_ssim, None);
        assert!(serde_json::from_str::<VmafLog>("{}").is_err());
    }

    #[test]
    fn frame_metrics_require_a_finite_value_on_every_scored_frame() {
        let log: VmafLog = serde_json::from_str(
            r#"{"frames":[
                {"frameNum":0,"metrics":{"vmaf":80.0,"float_ssim":0.8}},
                {"frameNum":2,"metrics":{"vmaf":100.0}}
            ]}"#,
        )
        .unwrap();

        assert_eq!(metric_sample(&log, |metrics| metrics.vmaf), Some(90.0));
        assert_eq!(metric_sample(&log, |metrics| metrics.float_ssim), None);
    }
}
