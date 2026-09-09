use super::{
    Ffmpeg,
    args::{Arg, ArgsExt},
    decoding::DecodingBackend,
    encoding::VideoEncoding,
    pixel_format,
};
use crate::{
    error::PlanError,
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

    pub async fn plan(
        &self,
        media: &MediaInfo,
        ffmpeg: &Ffmpeg,
    ) -> Result<TranscodePlan, PlanError> {
        let formats = match self.active_encoding(media) {
            Some(encoding)
                if !matches!(encoding, VideoEncoding::Vaapi { .. })
                    && self.decoding.direct_format(encoding).is_none() =>
            {
                ffmpeg
                    .encoder_help(encoding.name())
                    .await?
                    .pixel_formats
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        self.build(media, &formats)
    }

    fn active_encoding(&self, media: &MediaInfo) -> Option<&VideoEncoding> {
        match &self.video {
            VideoAction::Encode(encoding) if media.streams.iter().any(|s| s.is_regular_video()) => {
                Some(encoding)
            }
            _ => None,
        }
    }

    fn build(&self, media: &MediaInfo, formats: &[String]) -> Result<TranscodePlan, PlanError> {
        let mut input_args = Vec::new();
        let mut output_args = Vec::new();
        input_args.add(Arg::Overwrite(self.overwrite));
        output_args.extend([Arg::MapMetadata, Arg::MapChapters, Arg::CopyAll]);

        let encoding = self.active_encoding(media);
        if let Some(encoding) = encoding {
            output_args.add(Arg::VideoCodec(encoding.name()));
            encoding.append_options(&mut output_args);
            if let VideoEncoding::Vaapi { device, .. } = encoding
                && self.decoding.direct_format(encoding).is_none()
            {
                input_args.add(Arg::VaapiDevice(device));
            }
        }

        for (output_index, stream) in media.streams.iter().enumerate() {
            output_args.add(Arg::Map(stream.index));
            let Some(encoding) = encoding else { continue };

            if !stream.is_regular_video() {
                if stream.codec_type.as_deref() == Some("video") {
                    output_args.add(Arg::CopyStream(output_index));
                }
                continue;
            }

            let source =
                media
                    .pixel_format(stream)
                    .ok_or_else(|| PlanError::MissingPixelFormat {
                        stream_index: stream.index,
                        format: stream.pix_fmt.clone(),
                    })?;

            let target = if matches!(encoding, VideoEncoding::Vaapi { .. }) {
                pixel_format::vaapi_qsv_format(source)
            } else {
                pixel_format::encoder_format(media, source, formats)
            };

            let frame = self
                .decoding
                .plan(encoding, source, stream.index, output_index, target);

            input_args.extend(frame.input_args);
            output_args.extend(frame.output_args);
        }

        input_args.add(Arg::Input(&self.input));
        input_args.extend(output_args);
        input_args.extend([
            Arg::Format(self.container.muxer()),
            Arg::Output(&self.output),
        ]);

        Ok(TranscodePlan { args: input_args })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::encoding::{NvencMultipass, NvencPreset, RateControl, VideoCodec};

    fn descriptors() -> Vec<crate::ffprobe::types::PixelFormat> {
        serde_json::from_str::<MediaInfo>(include_str!("test_pixel_formats.json"))
            .unwrap()
            .pixel_formats
    }

    #[test]
    fn mapping_preserves_order_and_covers_override_shared_encoding() {
        let mut media: MediaInfo = serde_json::from_str(
            r#"{"streams":[
            {"index":4,"codec_type":"video","disposition":{"attached_pic":1}},
            {"index":1,"codec_type":"audio"},
            {"index":9,"codec_type":"video","field_order":"tt","pix_fmt":"nv12"},
            {"index":7,"codec_type":"subtitle"},
            {"index":2,"codec_type":"video","pix_fmt":"nv12","disposition":{"still_image":1}}
        ]}"#,
        )
        .unwrap();
        media.pixel_formats = descriptors();
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
        .build(&media, &["nv12".into()])
        .unwrap();
        let maps: Vec<_> = plan
            .args()
            .windows(2)
            .filter(|pair| pair[0] == "-map")
            .map(|pair| pair[1].to_str().unwrap())
            .collect();
        assert_eq!(maps, ["0:4", "0:1", "0:9", "0:7", "0:2"]);
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
                ("-c:0", "copy"),
            ]
        );
        assert!(
            !plan
                .args()
                .iter()
                .any(|arg| arg.to_str().unwrap().starts_with("-filter:"))
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
            .build(&media, &["nv12".into()])
            .unwrap();
        assert_eq!(
            plan.args(),
            [
                "-y",
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
                r#"[{"index":0,"codec_type":"video","pix_fmt":"yuv420p"},{"index":1,"codec_type":"video","pix_fmt":"yuv420p10le"}]"#,
                1,
            ),
        ] {
            let mut media: MediaInfo =
                serde_json::from_str(&format!(r#"{{"streams":{streams}}}"#)).unwrap();
            media.pixel_formats = descriptors();
            let plan = TranscodeRequest::mkv("input", "output")
                .with_video(VideoAction::encode_vaapi(
                    VideoCodec::Av1,
                    "/dev/dri/custom",
                    Some(RateControl::Quality(28)),
                ))
                .build(&media, &["nv12".into()])
                .unwrap();
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
    #[test]
    fn frame_paths_cover_all_decoder_encoder_pairs_and_device_boundaries() {
        let mut media: MediaInfo = serde_json::from_str(
            r#"{"streams":[
            {"index":5,"codec_type":"audio"},
            {"index":9,"codec_type":"video","pix_fmt":"yuv420p10le"},
            {"index":3,"codec_type":"video","disposition":{"attached_pic":1}}
        ]}"#,
        )
        .unwrap();
        media.pixel_formats = descriptors();
        let decoders = [
            DecodingBackend::Software,
            DecodingBackend::Vaapi(None),
            DecodingBackend::Cuda(None),
            DecodingBackend::Qsv(None),
        ];
        let encoders = [
            VideoAction::encode_x264(None, None),
            VideoAction::encode_vaapi(VideoCodec::Hevc, "/dev/dri/renderD128", None),
            VideoAction::encode_nvenc(VideoCodec::Hevc, None, None, None),
            VideoAction::encode_qsv(VideoCodec::Hevc, None, None),
        ];
        for (d, decoding) in decoders.iter().enumerate() {
            for (e, video) in encoders.iter().enumerate() {
                let formats = if e == 0 {
                    vec!["yuv420p10le".into()]
                } else {
                    vec!["p010le".into()]
                };
                let plan = TranscodeRequest::mkv("input", "output")
                    .with_video(video.clone())
                    .with_decoding(decoding.clone())
                    .build(&media, &formats)
                    .unwrap();
                let value = |key: &str| {
                    plan.args()
                        .windows(2)
                        .find(|p| p[0] == key)
                        .map(|p| p[1].to_str().unwrap())
                };
                let direct = d != 0 && d == e;
                assert_eq!(
                    value("-pix_fmt:1"),
                    Some(if direct {
                        ["", "+vaapi", "+cuda", "+qsv"][d]
                    } else if e == 1 {
                        "+vaapi"
                    } else if e == 0 {
                        "+yuv420p10le"
                    } else {
                        "+p010le"
                    })
                );
                let filter = value("-filter:1").unwrap_or("");
                assert_eq!(
                    filter.contains("hwdownload"),
                    d != 0 && !direct,
                    "d={d} e={e}: {filter}"
                );
                assert_eq!(
                    filter.contains("hwupload"),
                    e == 1 && !direct,
                    "d={d} e={e}: {filter}"
                );
                assert_eq!(value("-vaapi_device").is_some(), e == 1 && !direct);
                if direct {
                    assert!(filter.is_empty());
                }
                assert_eq!(value("-c:2"), Some("copy"));
                assert!(value("-pix_fmt:2").is_none());
                assert!(value("-hwaccel:3").is_none());
                let input = plan.args().iter().position(|a| a == "-i").unwrap();
                assert!(plan.args()[..input].iter().any(|a| a == "-hwaccel:9"));
            }
        }
        for device in [
            None,
            Some("/dev/dri/renderD128"),
            Some("/dev/dri/renderD129"),
        ] {
            let plan = TranscodeRequest::mkv("input", "output")
                .with_video(encoders[1].clone())
                .with_decoding(DecodingBackend::Vaapi(device.map(OsString::from)))
                .build(&media, &[])
                .unwrap();
            let value = |key: &str| {
                plan.args()
                    .windows(2)
                    .find(|p| p[0] == key)
                    .map(|p| p[1].to_str().unwrap())
            };
            assert_eq!(
                value("-hwaccel_device:9"),
                Some(device.unwrap_or("/dev/dri/renderD128"))
            );
            assert_eq!(
                value("-filter:1"),
                if device == Some("/dev/dri/renderD129") {
                    Some("hwdownload,format=p010le,hwupload")
                } else {
                    None
                }
            );
        }
    }

    #[test]
    fn only_equivalent_layouts_are_selected_and_ffmpeg_owns_rejection() {
        for (input, supported, target, filter) in [
            (
                "yuv420p10le",
                "p010le",
                "+p010le",
                Some("format=yuv420p10le,scale=iw:ih,format=p010le"),
            ),
            ("yuv444p12le", "yuv420p10le", "+yuv444p12le", None),
            ("rgb24", "yuv444p", "+rgb24", None),
            (
                "rgb24",
                "gbrp",
                "+gbrp",
                Some("format=rgb24,scale=iw:ih,format=gbrp"),
            ),
            ("rgb565le", "rgb24", "+rgb565le", None),
            (
                "rgb565le",
                "rgb565be",
                "+rgb565be",
                Some("format=rgb565le,scale=iw:ih,format=rgb565be"),
            ),
            ("pal8", "yuv420p", "+pal8", None),
            ("yuva420p", "yuv420p", "+yuva420p", None),
            ("gray", "yuv420p", "+gray", None),
        ] {
            let mut media: MediaInfo = serde_json::from_str(&format!(
                r#"{{"streams":[{{"index":0,"codec_type":"video","pix_fmt":"{input}"}}]}}"#
            ))
            .unwrap();
            media.pixel_formats = descriptors();
            let plan = TranscodeRequest::mkv("input", "output")
                .with_video(VideoAction::encode_x264(None, None))
                .build(&media, &[supported.into()])
                .unwrap();
            let value = |key: &str| {
                plan.args()
                    .windows(2)
                    .find(|p| p[0] == key)
                    .map(|p| p[1].to_str().unwrap())
            };
            assert_eq!(value("-pix_fmt:0"), Some(target));
            assert_eq!(value("-filter:0"), filter);
        }
    }
}
