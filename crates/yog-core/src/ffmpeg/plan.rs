use super::{args::Arg, decoding::DecodingBackend, encoding::VideoEncoding};
use crate::{
    ffmpeg::encoding::{NvencMultipass, NvencPreset, Preset, QsvPreset, RateControl, VideoCodec},
    ffprobe::types::MediaInfo,
};
use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Container {
    #[default]
    #[cfg_attr(feature = "clap", value(name = "mkv"))]
    Matroska,
    Mp4,
    Mov,
    Webm,
    #[cfg_attr(feature = "clap", value(name = "ts"))]
    MpegTs,
}

impl Container {
    pub(super) fn muxer(self) -> &'static str {
        match self {
            Self::Matroska => "matroska",
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Webm => "webm",
            Self::MpegTs => "mpegts",
        }
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "clap", derive(clap::Subcommand))]
pub enum VideoAction {
    #[default]
    #[cfg_attr(feature = "clap", command(long_flag = "copy"))]
    Copy,
    #[cfg_attr(feature = "clap", command(flatten))]
    Encode(VideoEncoding),
}

impl VideoAction {
    pub fn encode_x264(rate: Option<RateControl>, preset: Option<Preset>) -> Self {
        Self::Encode(VideoEncoding::X264 { rate, preset })
    }
    pub fn encode_x265(rate: Option<RateControl>, preset: Option<Preset>) -> Self {
        Self::Encode(VideoEncoding::X265 { rate, preset })
    }

    pub fn encode_svt_av1(rate: Option<RateControl>, preset: Option<u8>) -> Self {
        Self::Encode(VideoEncoding::SvtAv1 { rate, preset })
    }

    pub fn encode_aom_av1(rate: Option<RateControl>, cpu_used: Option<u8>) -> Self {
        Self::Encode(VideoEncoding::AomAv1 { rate, cpu_used })
    }

    pub fn encode_rav1e(rate: Option<RateControl>, speed: Option<u8>) -> Self {
        Self::Encode(VideoEncoding::Rav1e { rate, speed })
    }

    pub fn encode_nvenc(
        codec: VideoCodec,
        rate: Option<RateControl>,
        preset: Option<NvencPreset>,
        multipass: Option<NvencMultipass>,
    ) -> Self {
        Self::Encode(VideoEncoding::Nvenc {
            codec,
            rate,
            preset,
            multipass,
        })
    }

    pub fn encode_qsv(
        codec: VideoCodec,
        rate: Option<RateControl>,
        preset: Option<QsvPreset>,
    ) -> Self {
        Self::Encode(VideoEncoding::Qsv {
            codec,
            rate,
            preset,
        })
    }

    pub fn encode_vaapi(
        codec: VideoCodec,
        device: impl Into<PathBuf>,
        rate: Option<RateControl>,
    ) -> Self {
        Self::Encode(VideoEncoding::Vaapi {
            codec,
            rate,
            device: device.into(),
        })
    }
}

#[derive(Debug, Default)]
pub struct TranscodeRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub container: Container,
    pub video: VideoAction,
    pub decoding: DecodingBackend,
    pub overwrite: bool,
}

#[derive(Debug)]
pub struct TranscodePlan {
    args: Vec<OsString>,
}

impl TranscodePlan {
    pub fn args(&self) -> &[OsString] {
        &self.args
    }
}

impl TranscodeRequest {
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        container: Container,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            container,
            ..Self::default()
        }
    }

    pub fn mkv(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output, Container::Matroska)
    }

    pub fn mp4(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output, Container::Mp4)
    }

    pub fn mov(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output, Container::Mov)
    }

    pub fn webm(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output, Container::Webm)
    }

    pub fn mpeg_ts(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output, Container::MpegTs)
    }

    pub fn with_input(mut self, input: impl Into<PathBuf>) -> Self {
        self.input = input.into();
        self
    }

    pub fn with_output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = output.into();
        self
    }

    pub fn with_container(mut self, container: Container) -> Self {
        self.container = container;
        self
    }

    pub fn with_video(mut self, video: VideoAction) -> Self {
        self.video = video;
        self
    }

    pub fn with_decoding(mut self, decoding: DecodingBackend) -> Self {
        self.decoding = decoding;
        self
    }

    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    pub fn plan(&self, media: &MediaInfo) -> TranscodePlan {
        let mut args = Vec::new();
        Arg::Overwrite(self.overwrite).append_to(&mut args);
        let mut streams: Vec<_> = media.streams.iter().collect();
        streams.sort_by_key(|stream| stream.index);
        let is_normal_video = |stream: &&crate::ffprobe::types::MediaStream| {
            stream.codec_type.as_deref() == Some("video")
                && ["attached_pic", "timed_thumbnails", "still_image"]
                    .iter()
                    .all(|key| stream.disposition.get(*key).copied().unwrap_or(0) == 0)
        };
        if streams.iter().any(is_normal_video)
            && let VideoAction::Encode(VideoEncoding::Vaapi { device, .. }) = &self.video
        {
            Arg::VaapiDevice(device).append_to(&mut args);
        }
        self.decoding.append_to(&mut args);
        Arg::Input(&self.input).append_to(&mut args);
        for stream in &streams {
            Arg::Map(stream.index).append_to(&mut args);
        }
        Arg::MapMetadata.append_to(&mut args);
        Arg::MapChapters.append_to(&mut args);
        let mut video_ordinal = 0;
        for (output_index, stream) in streams.iter().enumerate() {
            if is_normal_video(stream)
                && let VideoAction::Encode(encoding) = &self.video
            {
                Arg::Codec {
                    index: output_index,
                    name: encoding.name(),
                }
                .append_to(&mut args);
                encoding.append_options(video_ordinal, &mut args);
            } else {
                Arg::Codec {
                    index: output_index,
                    name: "copy",
                }
                .append_to(&mut args);
            }
            if stream.codec_type.as_deref() == Some("video") {
                video_ordinal += 1;
            }
        }
        Arg::Format(self.container.muxer()).append_to(&mut args);
        Arg::Output(&self.output).append_to(&mut args);
        TranscodePlan { args }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::encoding::{NvencMultipass, NvencPreset, RateControl, VideoCodec};

    #[test]
    fn mapping_preserves_cover_and_uses_output_video_ordinals() {
        let media: MediaInfo = serde_json::from_str(
            r#"{"streams":[
            {"index":9,"codec_type":"video","field_order":"tt","pix_fmt":"nv12"},
            {"index":1,"codec_type":"audio"},
            {"index":4,"codec_type":"video","disposition":{"attached_pic":1}},
            {"index":7,"codec_type":"subtitle"}
        ]}"#,
        )
        .unwrap();
        let plan = TranscodeRequest {
            input: PathBuf::from("input.mkv"),
            output: PathBuf::from("-output.mkv"),
            container: Container::Matroska,
            overwrite: false,
            decoding: DecodingBackend::default(),
            video: VideoAction::Encode(VideoEncoding::Nvenc {
                codec: VideoCodec::Hevc,
                rate: Some(RateControl::Quality(23)),
                preset: Some(NvencPreset::P4),
                multipass: Some(NvencMultipass::FullResolution),
            }),
        }
        .plan(&media);
        // Full command asserts order and absence of implicit transforms/progress flags.
        assert_eq!(
            plan.args(),
            [
                "-n",
                "-hwaccel",
                "none",
                "-i",
                "input.mkv",
                "-map",
                "0:1",
                "-map",
                "0:4",
                "-map",
                "0:7",
                "-map",
                "0:9",
                "-map_metadata",
                "0",
                "-map_chapters",
                "0",
                "-c:0",
                "copy",
                "-c:1",
                "copy",
                "-c:2",
                "copy",
                "-c:3",
                "hevc_nvenc",
                "-rc:v:1",
                "vbr",
                "-b:v:1",
                "0",
                "-cq:v:1",
                "23",
                "-preset:v:1",
                "p4",
                "-multipass:v:1",
                "fullres",
                "-f",
                "matroska",
                "-output.mkv"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn copy_has_no_encoding_options_and_does_not_drop_unknown_streams() {
        let media: MediaInfo = serde_json::from_str(r#"{"streams":[{"index":0,"codec_type":"audio"},{"index":2,"codec_type":"data","codec_name":"unknown"}]}"#).unwrap();
        let plan = TranscodeRequest::default()
            .with_input("in.ts")
            .with_output("out.webm")
            .with_container(Container::Webm)
            .with_overwrite(true)
            .plan(&media);
        assert_eq!(
            plan.args(),
            [
                "-y",
                "-hwaccel",
                "none",
                "-i",
                "in.ts",
                "-map",
                "0:0",
                "-map",
                "0:2",
                "-map_metadata",
                "0",
                "-map_chapters",
                "0",
                "-c:0",
                "copy",
                "-c:1",
                "copy",
                "-f",
                "webm",
                "out.webm"
            ]
            .map(OsString::from)
        );
    }
}
