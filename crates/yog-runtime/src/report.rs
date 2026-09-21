pub mod record;
pub mod transcode;

use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::Serialize;
use yog_core::{ffmpeg::plan::TranscodeRequest, ffprobe::types::MediaInfo};

use crate::{
    diagnostics::Diagnostics,
    report::record::{EmulateRecord, PredictRecord, SkippedRecord, TranscodeRecord},
};

trait ToAbsolute {
    fn to_absolute(&self) -> String;
}

impl ToAbsolute for Path {
    fn to_absolute(&self) -> String {
        self.canonicalize()
            .or_else(|_| std::path::absolute(self))
            .unwrap_or_else(|_| self.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Record {
    Transcode(TranscodeRecord),
    Predict(PredictRecord),
    Emulate(EmulateRecord),
    Skipped(SkippedRecord),
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Success,
    Failure,
    Cancelled,
}

pub struct Report {
    path: Option<PathBuf>,
    file: Option<BufWriter<File>>,
}

impl TryFrom<Option<&Path>> for Report {
    type Error = anyhow::Error;

    fn try_from(value: Option<&Path>) -> Result<Self, Self::Error> {
        let Some(path) = value else {
            return Ok(Self {
                path: None,
                file: None,
            });
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create report directory {}", parent.display()))?;
        }
        let file = File::create(path)
            .with_context(|| format!("cannot create report {}", path.display()))?;

        Ok(Self {
            path: Some(path.to_path_buf()),
            file: Some(BufWriter::new(file)),
        })
    }
}

impl Report {
    pub fn write(&mut self, record: &Record, diagnostics: &Diagnostics<'_>) {
        let error = match self.file.as_mut() {
            Some(file) => file.write_line(record).err(),
            None => None,
        };
        if let Some(error) = error {
            self.warn(diagnostics, &error);
        }
    }

    pub fn finish(&mut self, diagnostics: &Diagnostics<'_>) {
        let error = match self.file.as_mut() {
            Some(file) => file.flush().err(),
            None => None,
        };
        if let Some(error) = error {
            self.warn(diagnostics, &error);
        }
    }

    fn warn(&mut self, diagnostics: &Diagnostics<'_>, error: &io::Error) {
        diagnostics.report_error(self.path.as_deref(), error);
    }
}

trait WriteLine {
    fn write_line(&mut self, record: &Record) -> io::Result<()>;
}

impl WriteLine for BufWriter<File> {
    fn write_line(&mut self, record: &Record) -> io::Result<()> {
        let line = serde_json::to_string(record).map_err(io::Error::other)?;
        self.write_all(line.as_bytes())?;
        self.write_all(b"\n")?;
        self.flush()
    }
}
fn container(request: &TranscodeRequest, media: &MediaInfo) -> Option<String> {
    request
        .output_container(media)
        .ok()
        .map(|container| container.extension().to_owned())
}

fn percent(value: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| value as f64 / total as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, time::Duration};

    use crate::report::record::{
        EmulateRecord, EstimateRecord, OutputsRecord, PredictRecord, TaskRecord, TranscodeRecord,
    };

    use super::*;
    use serde_json::{Value, json};
    use yog_core::ffmpeg::{
        encoding::{Preset, RateControl, VideoCodec, VideoEncoding},
        plan::{TranscodeRequest, VideoAction},
        prediction::{Estimate, PredictionOptions},
    };

    fn source(media: &str) -> MediaInfo {
        serde_json::from_str(media).unwrap()
    }

    fn value(record: &Record) -> Value {
        serde_json::to_value(record).unwrap()
    }

    #[test]
    fn copy_and_encoding_records_name_their_options() {
        let copy = value(&Record::Transcode(TranscodeRecord::new(
            &TranscodeRequest::mp4("in.mkv", "out.mp4"),
        )));
        assert_eq!(copy["transcode"]["video"]["action"], "copy");
        assert_eq!(copy["transcode"]["video"]["encoder"], Value::Null);
        assert_eq!(copy["transcode"]["video"]["rate"], Value::Null);
        assert!(
            copy["transcode"]["input"]
                .as_str()
                .unwrap()
                .ends_with("in.mkv")
        );

        let x264 = TranscodeRecord::new(&TranscodeRequest::mkv("in.mkv", "out.mkv").with_video(
            VideoAction::Encode(VideoEncoding::X264 {
                rate: Some(RateControl::Quality(23)),
                preset: Some(Preset::Medium),
            }),
        ));
        let x264 = value(&Record::Transcode(x264));
        assert_eq!(
            x264["transcode"]["video"]["rate"],
            json!({"kind": "quality", "parameter": "CRF", "value": 23})
        );
        assert_eq!(x264["transcode"]["video"]["preset"], "medium");
        assert_eq!(x264["transcode"]["video"]["device"], Value::Null);

        let vaapi = TranscodeRecord::new(&TranscodeRequest::mkv("in.mkv", "out.mkv").with_video(
            VideoAction::Encode(VideoEncoding::Vaapi {
                codec: VideoCodec::Av1,
                rate: Some(RateControl::Bitrate(2_000_000.try_into().unwrap())),
                device: "/dev/dri/renderD128".into(),
            }),
        ));
        let vaapi = value(&Record::Transcode(vaapi));
        assert_eq!(
            vaapi["transcode"]["video"]["rate"],
            json!({"kind": "bitrate", "parameter": "b", "value": 2_000_000})
        );
        assert_eq!(vaapi["transcode"]["video"]["preset"], Value::Null);
        assert_eq!(vaapi["transcode"]["video"]["device"], "/dev/dri/renderD128");

        let nvenc = TranscodeRecord::new(&TranscodeRequest::mkv("in.mkv", "out.mkv").with_video(
            VideoAction::encode_nvenc(
                VideoCodec::Hevc,
                Some(RateControl::Quality(28)),
                Some(yog_core::ffmpeg::encoding::NvencPreset::P5),
                Some(yog_core::ffmpeg::encoding::NvencMultipass::FullResolution),
            ),
        ));
        let nvenc = value(&Record::Transcode(nvenc));
        assert_eq!(nvenc["transcode"]["video"]["rate"]["parameter"], "CQ");
        assert_eq!(nvenc["transcode"]["video"]["preset"], "p5");
        assert_eq!(nvenc["transcode"]["video"]["multipass"], "fullres");
    }

    #[test]
    fn source_records_count_covers_separately_and_keep_raw_units() {
        let media = source(
            r#"{"streams":[
                {"index":0,"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","width":1920,"height":1080,"avg_frame_rate":"24000/1001","bit_rate":"1500000"},
                {"index":1,"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000"},
                {"index":2,"codec_type":"video","codec_name":"png","disposition":{"attached_pic":1}},
                {"index":3,"codec_type":"subtitle"},
                {"index":4,"codec_type":"attachment"},
                {"index":5,"codec_type":"data"}
            ],"format":{"format_name":"matroska,webm","duration":"12.5","start_time":"0"}}"#,
        );
        let request = TranscodeRequest::mkv("in.mkv", "out.mkv");
        let mut record = TranscodeRecord::new(&request);
        record.fill_source(&request, &media);
        let record = value(&Record::Transcode(record));

        assert_eq!(record["transcode"]["container"], "mkv");
        let source = &record["transcode"]["source"];
        assert_eq!(source["duration_seconds"], 12.5);
        assert_eq!(
            source["streams"],
            json!({"video":1,"audio":1,"subtitle":1,"attachment":1,"cover":1,"other":1})
        );
        assert_eq!(source["video"]["frame_rate"], "24000/1001");
        assert_eq!(source["video"]["bit_rate"], "1500000");
        assert_eq!(
            source["audio"],
            json!([{"codec":"aac","channels":2,"sample_rate":"48000"}])
        );
    }

    #[test]
    fn predictions_are_reported_with_their_sampling_configuration() {
        let media = source(
            r#"{"streams":[{"index":3,"codec_type":"video","width":320,"height":180,"pix_fmt":"yuv420p"}],
                "format":{"format_name":"matroska,webm","duration":"10"}}"#,
        );
        let request =
            TranscodeRequest::new("in.mkv", "").with_video(VideoAction::encode_x264(None, None));
        let options = PredictionOptions {
            samples: NonZeroUsize::new(5).unwrap(),
            sample_duration: Duration::from_secs_f64(2.0),
        };
        let mut record = PredictRecord::new(&request, options);
        record.fill_source(&request, &media);
        let record = value(&Record::Predict(record));

        assert_eq!(
            record["predict"]["sampling"],
            json!({"requested_samples":5,"sample_seconds":2.0,"measured_samples":0,"sampled_seconds":0.0})
        );
        assert_eq!(record["predict"]["speed"], Value::Null);
        assert_eq!(record["predict"]["samples"], json!([]));
        assert_eq!(record["predict"]["status"], "success");
    }

    #[test]
    fn estimate_records_mirror_value_low_high() {
        let record = value(&Record::Predict(PredictRecord::new(
            &TranscodeRequest::new("in.mkv", "").with_video(VideoAction::encode_x264(None, None)),
            PredictionOptions {
                samples: std::num::NonZeroUsize::new(1).unwrap(),
                sample_duration: std::time::Duration::from_secs(1),
            },
        )));
        assert_eq!(record["predict"]["transcode_seconds"], Value::Null);

        let estimate = EstimateRecord::from(Estimate {
            value: 3.0,
            low: 1.0,
            high: 9.0,
        });
        assert_eq!(
            serde_json::to_value(estimate).unwrap(),
            json!({"value":3.0,"low":1.0,"high":9.0})
        );
    }

    #[test]
    fn skipped_records_keep_the_probe_failure() {
        let record = Record::Skipped(SkippedRecord::new(
            Path::new("input/notes.md"),
            &anyhow::anyhow!("skipping because ffprobe failed: not media"),
        ));
        let record = value(&record);
        assert!(
            record["skipped"]["input"]
                .as_str()
                .unwrap()
                .ends_with("notes.md")
        );
        assert_eq!(
            record["skipped"]["reason"],
            "skipping because ffprobe failed: not media"
        );
    }

    #[test]
    fn emulate_records_carry_the_quality_point_and_chart_paths() {
        let request = TranscodeRequest::new("in.mkv", "")
            .with_video(VideoAction::encode_x264(None, Some(Preset::Medium)));
        let options = PredictionOptions {
            samples: NonZeroUsize::new(5).unwrap(),
            sample_duration: Duration::from_secs_f64(2.0),
        };
        let outputs = OutputsRecord::new(Some(Path::new("quality.png")), None);
        let record = EmulateRecord::new(&request, 23, None, outputs.clone(), options);
        let record = value(&Record::Emulate(record));

        let record = &record["emulate"];
        assert!(record.get("predict").is_none(), "{record}");
        assert!(record["input"].as_str().unwrap().starts_with('/'));
        assert_eq!(record["status"], "success");
        assert_eq!(record["error"], Value::Null);
        assert_eq!(record["candidate_index"], Value::Null);
        assert_eq!(record["decoding"], "software");
        assert_eq!(
            record["video"]["rate"],
            json!({"kind":"quality","parameter":"CRF","value":23})
        );
        assert_eq!(record["video"]["preset"], "medium");
        let png = record["outputs"]["png"].as_str().unwrap();
        assert!(png.starts_with('/'), "{png}");
        assert!(png.ends_with("quality.png"), "{png}");
        assert_eq!(record["outputs"]["svg"], Value::Null);
        assert_eq!(record["sampling"]["requested_samples"], 5);
        assert_eq!(record["quality"], Value::Null);
        assert_eq!(record["samples"], json!([]));

        let candidate_request = TranscodeRequest::new("in.mkv", "").with_video(
            VideoAction::Encode(VideoEncoding::Vaapi {
                codec: VideoCodec::Hevc,
                rate: None,
                device: "/dev/dri/renderD129".into(),
            }),
        );
        let failed = EmulateRecord::new(&candidate_request, 30, Some(2), outputs, options)
            .finish(Status::Failure, Some("boom".to_owned()));
        let failed = value(&failed);
        let failed = &failed["emulate"];
        assert_eq!(failed["status"], "failure");
        assert_eq!(failed["error"], "boom");
        assert_eq!(failed["video"]["rate"]["value"], 30);
        assert_eq!(failed["outputs"]["svg"], Value::Null);
        assert_eq!(failed["candidate_index"], 2);
        assert_eq!(failed["video"]["encoder"], "hevc_vaapi");
        assert_eq!(failed["video"]["device"], "/dev/dri/renderD129");
    }
}
