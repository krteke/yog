use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};

use crate::error::ProbeValueError;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaInfo {
    #[serde(default)]
    pub streams: Vec<MediaStream>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    #[serde(default)]
    pub programs: Vec<serde_json::Value>,
    #[serde(default)]
    pub format: MediaFormat,
    #[serde(default)]
    pub pixel_formats: Vec<PixelFormat>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MediaFormat {
    pub filename: Option<String>,
    pub format_name: Option<String>,
    pub format_long_name: Option<String>,
    pub start_time: Option<String>,
    pub duration: Option<String>,
    pub size: Option<String>,
    pub bit_rate: Option<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaStream {
    pub index: usize,
    pub codec_name: Option<String>,
    pub codec_long_name: Option<String>,
    pub profile: Option<String>,
    pub codec_type: Option<String>,
    pub codec_tag_string: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_aspect_ratio: Option<String>,
    pub display_aspect_ratio: Option<String>,
    pub pix_fmt: Option<String>,
    pub level: Option<i64>,
    pub color_range: Option<String>,
    pub color_space: Option<String>,
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
    pub chroma_location: Option<String>,
    pub field_order: Option<String>,
    pub r_frame_rate: Option<String>,
    pub avg_frame_rate: Option<String>,
    pub time_base: Option<String>,
    pub start_time: Option<String>,
    pub duration: Option<String>,
    pub bit_rate: Option<String>,
    pub sample_fmt: Option<String>,
    pub sample_rate: Option<String>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
    pub bits_per_sample: Option<u32>,
    pub bits_per_raw_sample: Option<String>,
    pub extradata_size: Option<u64>,
    pub extradata_hash: Option<String>,
    #[serde(default)]
    pub disposition: BTreeMap<String, i32>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    #[serde(default)]
    pub side_data_list: Vec<SideData>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SideData {
    pub side_data_type: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Chapter {
    pub id: Option<i64>,
    pub time_base: Option<String>,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Frame {
    pub stream_index: Option<usize>,
    pub media_type: Option<String>,
    pub pts_time: Option<String>,
    pub best_effort_timestamp_time: Option<String>,
    pub duration_time: Option<String>,
    pub pix_fmt: Option<String>,
    pub color_range: Option<String>,
    pub color_space: Option<String>,
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
    pub chroma_location: Option<String>,
    pub interlaced_frame: Option<i32>,
    pub top_field_first: Option<i32>,
    #[serde(default)]
    pub side_data_list: Vec<SideData>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Packet {
    pub stream_index: usize,
    pub pts_time: Option<String>,
    pub dts_time: Option<String>,
    pub duration_time: Option<String>,
    pub size: Option<String>,
    pub flags: Option<String>,
}

impl MediaFormat {
    pub fn try_duration(&self) -> Result<Duration, ProbeValueError> {
        let value = self.duration.as_deref().ok_or(ProbeValueError::Missing {
            field: "format duration",
        })?;
        parse_duration(value, "format duration")
    }

    pub fn try_start_time(&self) -> Result<Option<f64>, ProbeValueError> {
        parse_optional_seconds(self.start_time.as_deref(), "format start time")
    }
}

impl Packet {
    pub fn try_size(&self) -> Result<u64, ProbeValueError> {
        let value = self.size.as_deref().ok_or(ProbeValueError::Missing {
            field: "packet size",
        })?;
        value.parse().map_err(|_| ProbeValueError::Invalid {
            field: "packet size",
            value: value.to_owned(),
        })
    }
}

impl MediaStream {
    pub fn try_duration(&self) -> Result<Option<Duration>, ProbeValueError> {
        let Some(value) = optional_probe_value(self.duration.as_deref()) else {
            return Ok(None);
        };
        parse_duration(value, "stream duration").map(Some)
    }

    pub fn try_start_time(&self) -> Result<Option<f64>, ProbeValueError> {
        parse_optional_seconds(self.start_time.as_deref(), "stream start time")
    }

    pub fn is_regular_video(&self) -> bool {
        self.codec_type.as_deref() == Some("video")
            && self.disposition.get("attached_pic").copied().unwrap_or(0) == 0
    }
}

fn parse_duration(value: &str, field: &'static str) -> Result<Duration, ProbeValueError> {
    let duration = Duration::try_from_secs_f64(parse_seconds(value, field)?)
        .map_err(|_| invalid_value(field, value))?;
    if duration.is_zero() {
        return Err(invalid_value(field, value));
    }
    Ok(duration)
}

fn parse_optional_seconds(
    value: Option<&str>,
    field: &'static str,
) -> Result<Option<f64>, ProbeValueError> {
    optional_probe_value(value)
        .map(|value| parse_seconds(value, field))
        .transpose()
}

fn optional_probe_value(value: Option<&str>) -> Option<&str> {
    value.filter(|value| *value != "N/A")
}

fn parse_seconds(value: &str, field: &'static str) -> Result<f64, ProbeValueError> {
    value
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite())
        .ok_or_else(|| invalid_value(field, value))
}

fn invalid_value(field: &'static str, value: &str) -> ProbeValueError {
    ProbeValueError::Invalid {
        field,
        value: value.to_owned(),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PixelFormat {
    pub name: String,
    pub nb_components: u8,
    pub log2_chroma_w: Option<u8>,
    pub log2_chroma_h: Option<u8>,
    pub flags: PixelFormatFlags,
    #[serde(default)]
    pub components: Vec<PixelComponent>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PixelFormatFlags {
    pub rgb: u8,
    pub alpha: u8,
    pub palette: u8,
    pub hwaccel: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PixelComponent {
    pub bit_depth: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_preserves_wire_values_and_heterogeneous_side_data() {
        let media: MediaInfo = serde_json::from_str(
            r#"{"streams":[{
            "index":2,"codec_type":"audio","sample_rate":"48000","duration":"N/A",
            "avg_frame_rate":"0/0","side_data_list":[{
                "side_data_type":"Mastering display metadata","red_x":"34000/50000"
            }]}]}"#,
        )
        .unwrap();
        assert_eq!(media.streams[0].duration.as_deref(), Some("N/A"));
        assert_eq!(media.streams[0].avg_frame_rate.as_deref(), Some("0/0"));
        assert_eq!(media.streams[0].try_duration().unwrap(), None);
        assert_eq!(
            media.streams[0].side_data_list[0].extra["red_x"],
            "34000/50000"
        );
        assert!(media.format.duration.is_none());
        assert!(serde_json::from_str::<MediaInfo>(r#"{"streams":[{}]}"#).is_err());
    }

    #[test]
    fn media_times_are_parsed_once_at_the_probe_boundary() {
        let mut media: MediaInfo = serde_json::from_str(
            r#"{
                "format":{"start_time":"10.5","duration":"100"},
                "streams":[{"index":3,"start_time":"20.25","duration":"30.5"}]
            }"#,
        )
        .unwrap();

        assert_eq!(media.format.try_start_time().unwrap(), Some(10.5));
        assert_eq!(
            media.format.try_duration().unwrap(),
            Duration::from_secs(100)
        );
        assert_eq!(media.streams[0].try_start_time().unwrap(), Some(20.25));
        assert_eq!(
            media.streams[0].try_duration().unwrap(),
            Some(Duration::from_secs_f64(30.5))
        );

        media.streams[0].duration = Some("inf".to_owned());
        assert!(matches!(
            media.streams[0].try_duration(),
            Err(ProbeValueError::Invalid {
                field: "stream duration",
                ..
            })
        ));
    }
}
