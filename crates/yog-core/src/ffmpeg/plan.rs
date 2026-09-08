use super::{
    args::{Arg, ArgsExt},
    decoding::DecodingBackend,
    encoding::VideoEncoding,
};
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
        args.add(Arg::Overwrite(self.overwrite));
        args.extend(self.decoding.args());
        args.extend([
            Arg::Input(&self.input),
            Arg::MapMetadata,
            Arg::MapChapters,
            Arg::CopyAll,
        ]);
        let mut has_video = false;
        let mut covers = Vec::new();
        for (output_index, stream) in media.streams.iter().enumerate() {
            args.add(Arg::Map(stream.index));
            if stream.codec_type.as_deref() == Some("video") {
                if stream.disposition.get("attached_pic").copied().unwrap_or(0) != 0 {
                    covers.push(output_index);
                } else {
                    has_video = true;
                }
            }
        }
        if has_video && let VideoAction::Encode(encoding) = &self.video {
            args.add(Arg::VideoCodec(encoding.name()));
            encoding.append_options(&mut args);
            if let VideoEncoding::Vaapi { device, .. } = encoding {
                args.add(Arg::VaapiDevice(device));
            }
            args.extend(covers.into_iter().map(Arg::CopyStream));
        }
        args.extend([
            Arg::Format(self.container.muxer()),
            Arg::Output(&self.output),
        ]);
        TranscodePlan { args }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::encoding::{NvencMultipass, NvencPreset, RateControl, VideoCodec};

    #[test]
    fn mapping_preserves_order_and_covers_override_shared_encoding() {
        let media: MediaInfo = serde_json::from_str(
            r#"{"streams":[
            {"index":9,"codec_type":"video","field_order":"tt","pix_fmt":"nv12"},
            {"index":1,"codec_type":"audio"},
            {"index":4,"codec_type":"video","disposition":{"attached_pic":1}},
            {"index":7,"codec_type":"subtitle"},
            {"index":2,"codec_type":"video","disposition":{"still_image":1}}
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
        let maps: Vec<_> = plan
            .args()
            .windows(2)
            .filter(|pair| pair[0] == "-map")
            .map(|pair| pair[1].to_str().unwrap())
            .collect();
        assert_eq!(maps, ["0:9", "0:1", "0:4", "0:7", "0:2"]);
        // Exact option sequence catches misplaced cover overrides and per-video duplication.
        let options: Vec<_> = plan
            .args()
            .windows(2)
            .filter(|pair| {
                pair[0] == "-c"
                    || pair[0].to_str().unwrap().starts_with("-c:")
                    || ["-rc:v", "-b:v", "-cq:v", "-preset:v", "-multipass:v"]
                        .contains(&pair[0].to_str().unwrap())
            })
            .map(|pair| (pair[0].to_str().unwrap(), pair[1].to_str().unwrap()))
            .collect();
        assert_eq!(
            options,
            [
                ("-c", "copy"),
                ("-c:v", "hevc_nvenc"),
                ("-rc:v", "vbr"),
                ("-b:v", "0"),
                ("-cq:v", "23"),
                ("-preset:v", "p4"),
                ("-multipass:v", "fullres"),
                ("-c:2", "copy"),
            ]
        );
        assert!(
            !plan
                .args()
                .iter()
                .any(|arg| ["-vf", "-pix_fmt", "-filter_complex"].contains(&arg.to_str().unwrap()))
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
                "-map_metadata",
                "0",
                "-map_chapters",
                "0",
                "-c",
                "copy",
                "-map",
                "0:0",
                "-map",
                "0:2",
                "-f",
                "webm",
                "out.webm"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn vaapi_device_is_only_emitted_once_when_encoding_a_video() {
        for (streams, devices) in [
            (
                r#"[{"index":0,"codec_type":"audio"},{"index":1,"codec_type":"video","disposition":{"attached_pic":1}}]"#,
                0,
            ),
            (
                r#"[{"index":0,"codec_type":"video"},{"index":1,"codec_type":"video"}]"#,
                1,
            ),
        ] {
            let media = serde_json::from_str(&format!(r#"{{"streams":{streams}}}"#)).unwrap();
            let plan = TranscodeRequest::mkv("input", "output")
                .with_video(VideoAction::encode_vaapi(
                    VideoCodec::Av1,
                    "/dev/dri/custom",
                    Some(RateControl::Quality(28)),
                ))
                .plan(&media);
            assert_eq!(
                plan.args()
                    .windows(2)
                    .filter(|pair| pair[0] == "-vaapi_device" && pair[1] == "/dev/dri/custom")
                    .count(),
                devices
            );
            for option in ["-c:v", "-rc_mode:v", "-qp:v"] {
                assert_eq!(
                    plan.args().iter().filter(|arg| *arg == option).count(),
                    devices
                );
            }
        }
    }
}
