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
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Container {
    #[cfg_attr(feature = "clap", value(name = "mkv"))]
    Matroska,
    Mp4,
    Mov,
    #[cfg_attr(feature = "clap", value(name = "m4a"))]
    M4a,
    #[cfg_attr(feature = "clap", value(name = "3gp"))]
    ThreeGp,
    #[cfg_attr(feature = "clap", value(name = "3g2"))]
    ThreeG2,
    #[cfg_attr(feature = "clap", value(name = "f4v"))]
    F4v,
    #[cfg_attr(feature = "clap", value(name = "ismv"))]
    Ismv,
    #[cfg_attr(feature = "clap", value(name = "psp"))]
    Psp,
    Webm,
    #[cfg_attr(feature = "clap", value(name = "ts"))]
    MpegTs,
    #[cfg_attr(feature = "clap", value(name = "m2ts"))]
    M2ts,
    Avi,
    Flv,
    Asf,
    Wmv,
    #[cfg_attr(feature = "clap", value(name = "mpg", alias = "mpeg"))]
    MpegPs,
    Vob,
    Ogg,
    Ogv,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Matroska => "mkv",
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::M4a => "m4a",
            Self::ThreeGp => "3gp",
            Self::ThreeG2 => "3g2",
            Self::F4v => "f4v",
            Self::Ismv => "ismv",
            Self::Psp => "psp",
            Self::Webm => "webm",
            Self::MpegTs => "ts",
            Self::M2ts => "m2ts",
            Self::Avi => "avi",
            Self::Flv => "flv",
            Self::Asf => "asf",
            Self::Wmv => "wmv",
            Self::MpegPs => "mpg",
            Self::Vob => "vob",
            Self::Ogg => "ogg",
            Self::Ogv => "ogv",
        }
    }

    pub(super) fn muxer(self) -> &'static str {
        match self {
            Self::Matroska => "matroska",
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::M4a => "ipod",
            Self::ThreeGp => "3gp",
            Self::ThreeG2 => "3g2",
            Self::F4v => "f4v",
            Self::Ismv => "ismv",
            Self::Psp => "psp",
            Self::Webm => "webm",
            Self::MpegTs | Self::M2ts => "mpegts",
            Self::Avi => "avi",
            Self::Flv => "flv",
            Self::Asf | Self::Wmv => "asf",
            Self::MpegPs => "mpeg",
            Self::Vob => "vob",
            Self::Ogg => "ogg",
            Self::Ogv => "ogv",
        }
    }

    pub fn from_input(path: &Path, format_name: Option<&str>) -> Option<Self> {
        let format_name = format_name?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        let has_format = |expected| {
            format_name
                .split(',')
                .any(|format| format.trim() == expected)
        };

        if has_format("mpegts") {
            return match extension.as_deref() {
                Some("ts") => Some(Self::MpegTs),
                Some("m2t" | "m2ts" | "mts") => Some(Self::M2ts),
                _ => Some(Self::MpegTs),
            };
        }
        if has_format("matroska") || has_format("webm") {
            return match extension.as_deref() {
                Some("mkv" | "mk3d" | "mka" | "mks") => Some(Self::Matroska),
                Some("webm") => Some(Self::Webm),
                _ => Some(Self::Matroska),
            };
        }
        if ["mov", "mp4", "m4a", "3gp", "3g2", "mj2"]
            .into_iter()
            .any(has_format)
        {
            return match extension.as_deref() {
                Some("mp4") => Some(Self::Mp4),
                Some("mov") => Some(Self::Mov),
                Some("m4a" | "m4b" | "m4v") => Some(Self::M4a),
                Some("3gp") => Some(Self::ThreeGp),
                Some("3g2") => Some(Self::ThreeG2),
                Some("f4v") => Some(Self::F4v),
                Some("ismv" | "isma") => Some(Self::Ismv),
                Some("psp") => Some(Self::Psp),
                _ => Some(Self::Mp4),
            };
        }
        if has_format("avi") {
            return Some(Self::Avi);
        }
        if has_format("flv") {
            return Some(Self::Flv);
        }
        if has_format("asf") {
            return Some(if extension.as_deref() == Some("wmv") {
                Self::Wmv
            } else {
                Self::Asf
            });
        }
        if has_format("mpeg") {
            return Some(if extension.as_deref() == Some("vob") {
                Self::Vob
            } else {
                Self::MpegPs
            });
        }
        if has_format("ogg") {
            return Some(if extension.as_deref() == Some("ogv") {
                Self::Ogv
            } else {
                Self::Ogg
            });
        }

        None
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

#[derive(Debug, Clone, Default)]
pub struct TranscodeRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub container: Option<Container>,
    pub video: VideoAction,
    pub decoding: DecodingBackend,
    pub overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct TranscodePlan {
    pub(super) input: PathBuf,
    pub(super) output: PathBuf,
    pub(super) args: Vec<OsString>,
    pub(super) attachments: Vec<PlannedAttachment>,
    input_position: usize,
}

#[derive(Debug, Clone)]
pub(super) struct PlannedAttachment {
    pub input_index: usize,
    pub output_index: usize,
    pub metadata: BTreeMap<String, String>,
    pub input: AttachmentInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttachmentInput {
    CoverFrame,
    AttachmentStream,
}

impl TranscodePlan {
    /// Expected tags for an input stream reattached by this plan. `None` means
    /// the stream uses the ordinary mapping, without attachment tag changes.
    pub fn attachment_metadata(&self, input_index: usize) -> Option<&BTreeMap<String, String>> {
        self.attachments
            .iter()
            .find(|attachment| attachment.input_index == input_index)
            .map(|attachment| &attachment.metadata)
    }

    pub(super) fn sample(
        &self,
        start: &str,
        duration: &str,
        output: PathBuf,
        include_covers: bool,
    ) -> Self {
        let mut plan = self.clone();
        let mut seek = Vec::with_capacity(2);
        seek.add(Arg::Seek(start));

        plan.args
            .splice(plan.input_position..plan.input_position, seek);
        plan.args.add(Arg::Duration(duration));

        for flag in ["-map_metadata", "-map_chapters"] {
            let position = plan
                .args
                .iter()
                .position(|argument| argument == flag)
                .expect("transcode plan contains metadata mapping");
            plan.args[position + 1] = "-1".into();
        }

        plan.output = output;

        let first_attachment_output = plan
            .attachments
            .first()
            .map(|attachment| attachment.output_index);

        plan.attachments
            .retain(|attachment| include_covers && attachment.input == AttachmentInput::CoverFrame);

        if let Some(first_attachment_output) = first_attachment_output {
            for (ordinal, attachment) in plan.attachments.iter_mut().enumerate() {
                attachment.output_index = first_attachment_output + ordinal;
            }
        }

        plan
    }
}

impl TranscodeRequest {
    pub fn new(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            ..Self::default()
        }
    }

    pub fn mkv(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Matroska)
    }

    pub fn mp4(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Mp4)
    }

    pub fn mov(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Mov)
    }

    pub fn m4a(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::M4a)
    }

    pub fn three_gp(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::ThreeGp)
    }

    pub fn three_g2(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::ThreeG2)
    }

    pub fn f4v(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::F4v)
    }

    pub fn ismv(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Ismv)
    }

    pub fn psp(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Psp)
    }

    pub fn webm(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Webm)
    }

    pub fn mpeg_ts(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::MpegTs)
    }

    pub fn m2ts(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::M2ts)
    }

    pub fn avi(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Avi)
    }

    pub fn flv(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Flv)
    }

    pub fn asf(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Asf)
    }

    pub fn wmv(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Wmv)
    }

    pub fn mpeg_ps(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::MpegPs)
    }

    pub fn vob(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Vob)
    }

    pub fn ogg(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Ogg)
    }

    pub fn ogv(input: impl Into<PathBuf>, output: impl Into<PathBuf>) -> Self {
        Self::new(input, output).with_container(Container::Ogv)
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
        self.container = Some(container);
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

    pub fn output_container(&self, media: &MediaInfo) -> Result<Container, PlanError> {
        self.container
            .or_else(|| Container::from_input(&self.input, media.format.format_name.as_deref()))
            .ok_or_else(|| PlanError::UnsupportedContainer {
                format: media.format.format_name.clone(),
            })
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
        let container = self.output_container(media)?;

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

        let mut attachments = Vec::new();
        let mut cover_ordinal = 0;
        let mut copied_timed_stream = false;
        let mut mapped_streams = 0;
        for stream in &media.streams {
            if matches!(container, Container::Matroska)
                && stream.codec_type.as_deref() == Some("video")
                && !stream.is_regular_video()
            {
                let mut metadata = stream.tags.clone();
                let image = match stream.codec_name.as_deref() {
                    Some("png") => Some(("png", "image/png")),
                    Some("mjpeg") => Some(("jpg", "image/jpeg")),
                    _ => None,
                };

                if let Some((extension, mime)) = image {
                    let filename = if cover_ordinal == 0 {
                        format!("cover.{extension}")
                    } else {
                        format!("cover-{cover_ordinal}.{extension}")
                    };
                    for (key, value) in [("filename", filename), ("mimetype", mime.to_owned())] {
                        if !metadata
                            .keys()
                            .any(|existing| existing.eq_ignore_ascii_case(key))
                        {
                            metadata.insert(key.to_owned(), value);
                        }
                    }
                }

                cover_ordinal += 1;
                attachments.push((stream.index, AttachmentInput::CoverFrame, metadata));
                continue;
            }

            if matches!(container, Container::Matroska)
                && encoding.is_some()
                && stream.codec_type.as_deref() == Some("attachment")
            {
                attachments.push((
                    stream.index,
                    AttachmentInput::AttachmentStream,
                    stream.tags.clone(),
                ));
                continue;
            }

            let output_index = mapped_streams;
            mapped_streams += 1;
            output_args.add(Arg::Map(stream.index));
            let Some(encoding) = encoding else { continue };

            if !stream.is_regular_video() {
                if stream.codec_type.as_deref() != Some("attachment") {
                    copied_timed_stream = true;
                }
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

        if matches!(container, Container::Matroska) && encoding.is_some() && copied_timed_stream {
            output_args.add(Arg::MaxInterleaveDelta(0));
        }
        if matches!(container, Container::M2ts) {
            output_args.add(Arg::M2tsMode);
        }

        let input_position = input_args.len();
        input_args.add(Arg::Input(&self.input));
        input_args.extend(output_args);
        input_args.add(Arg::Format(container.muxer()));
        let attachments = attachments
            .into_iter()
            .enumerate()
            .map(
                |(ordinal, (input_index, input, metadata))| PlannedAttachment {
                    input_index,
                    output_index: mapped_streams + ordinal,
                    metadata,
                    input,
                },
            )
            .collect();

        Ok(TranscodePlan {
            input: self.input.clone(),
            output: self.output.clone(),
            args: input_args,
            attachments,
            input_position,
        })
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
    fn input_container_uses_exact_extensions_and_family_fallbacks() {
        const MOV_FAMILY: &str = "mov,mp4,m4a,3gp,3g2,mj2";
        for (extension, format_name, container, muxer) in [
            ("mkv", "matroska,webm", Container::Matroska, "matroska"),
            ("WEBM", "matroska,webm", Container::Webm, "webm"),
            ("mp4", MOV_FAMILY, Container::Mp4, "mp4"),
            ("MOV", MOV_FAMILY, Container::Mov, "mov"),
            ("m4a", MOV_FAMILY, Container::M4a, "ipod"),
            ("m4b", MOV_FAMILY, Container::M4a, "ipod"),
            ("m4v", MOV_FAMILY, Container::M4a, "ipod"),
            ("3gp", MOV_FAMILY, Container::ThreeGp, "3gp"),
            ("3g2", MOV_FAMILY, Container::ThreeG2, "3g2"),
            ("f4v", MOV_FAMILY, Container::F4v, "f4v"),
            ("ISMV", MOV_FAMILY, Container::Ismv, "ismv"),
            ("isma", MOV_FAMILY, Container::Ismv, "ismv"),
            ("psp", MOV_FAMILY, Container::Psp, "psp"),
            ("ts", "mpegts", Container::MpegTs, "mpegts"),
            ("m2t", "mpegts", Container::M2ts, "mpegts"),
            ("m2ts", "mpegts", Container::M2ts, "mpegts"),
            ("mts", "mpegts", Container::M2ts, "mpegts"),
            ("avi", "avi", Container::Avi, "avi"),
            ("flv", "flv", Container::Flv, "flv"),
            ("asf", "asf", Container::Asf, "asf"),
            ("wmv", "asf", Container::Wmv, "asf"),
            ("mpg", "mpeg", Container::MpegPs, "mpeg"),
            ("mpeg", "mpeg", Container::MpegPs, "mpeg"),
            ("vob", "mpeg", Container::Vob, "vob"),
            ("ogg", "ogg", Container::Ogg, "ogg"),
            ("ogv", "ogg", Container::Ogv, "ogv"),
        ] {
            let path = format!("video.{extension}");
            let actual = Container::from_input(Path::new(&path), Some(format_name)).unwrap();
            assert_eq!(actual, container, "{path}");
            assert_eq!(actual.muxer(), muxer, "{path}");
        }
        for (path, format_name, container) in [
            ("video.mj2", MOV_FAMILY, Container::Mp4),
            ("video", MOV_FAMILY, Container::Mp4),
            ("video.bin", "mpegts", Container::MpegTs),
            ("video.bin", "matroska,webm", Container::Matroska),
            ("video.bin", "asf", Container::Asf),
            ("video.bin", "mpeg", Container::MpegPs),
            ("video.bin", "ogg", Container::Ogg),
            ("video.bin", "avi", Container::Avi),
        ] {
            assert_eq!(
                Container::from_input(Path::new(path), Some(format_name)),
                Some(container),
                "{path} {format_name}"
            );
        }
        assert!(Container::from_input(Path::new("video.nut"), Some("nut")).is_none());
        assert!(Container::from_input(Path::new("video.avi"), None).is_none());

        let media: MediaInfo =
            serde_json::from_str(r#"{"format":{"format_name":"mpegts"}}"#).unwrap();
        for (input, explicit, m2ts) in [
            ("video.m2ts", None, true),
            ("video.MTS", None, true),
            ("video.ts", None, false),
            ("video.m2ts", Some(Container::MpegTs), false),
            ("video.ts", Some(Container::M2ts), true),
        ] {
            let mut request = TranscodeRequest::new(input, "output");
            request.container = explicit;
            let plan = request.build(&media, &[]).unwrap();
            assert_eq!(
                plan.args.iter().any(|arg| arg == "-mpegts_m2ts_mode"),
                m2ts,
                "{input} {explicit:?}"
            );
        }
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
            container: Some(Container::Mp4),
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
            .args
            .windows(2)
            .filter(|pair| pair[0] == "-map")
            .map(|pair| pair[1].to_str().unwrap())
            .collect();
        assert_eq!(maps, ["0:4", "0:1", "0:9", "0:7", "0:2"]);
        // Exact option sequence catches misplaced cover overrides and per-video duplication.
        let options: Vec<_> = plan
            .args
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
                .args
                .iter()
                .any(|arg| arg.to_str().unwrap().starts_with("-filter:"))
        );
    }

    #[test]
    fn matroska_encoding_reattaches_native_attachments_and_bounds_interleaving() {
        let mut media: MediaInfo = serde_json::from_str(r#"{"streams":[
            {"index":8,"codec_type":"video","codec_name":"png","disposition":{"attached_pic":1},"tags":{"filename":"原封面.png","mimetype":"image/png","title":"Front","custom":"keep me"}},
            {"index":5,"codec_type":"audio"},
            {"index":9,"codec_type":"video","pix_fmt":"yuv420p10le"},
            {"index":15,"codec_type":"attachment","tags":{"filename":"font.ttf","mimetype":"font/ttf"}},
            {"index":12,"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},
            {"index":20,"codec_type":"video","pix_fmt":"yuv444p12le"}
        ]}"#).unwrap();
        media.pixel_formats = descriptors();
        let plan = TranscodeRequest::mkv("input", "output")
            .with_video(VideoAction::Copy)
            .build(&media, &[])
            .unwrap();
        let maps = || {
            plan.args
                .windows(2)
                .filter(|pair| pair[0] == "-map")
                .map(|pair| pair[1].to_str().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(maps(), ["0:5", "0:9", "0:15", "0:20"]);
        assert_eq!(
            plan.attachments
                .iter()
                .map(|attachment| (
                    attachment.input_index,
                    attachment.output_index,
                    attachment.input
                ))
                .collect::<Vec<_>>(),
            [
                (8, 4, AttachmentInput::CoverFrame),
                (12, 5, AttachmentInput::CoverFrame)
            ]
        );
        assert!(!plan.args.iter().any(|arg| arg == "-max_interleave_delta"));

        let plan = TranscodeRequest::mkv("input", "output")
            .with_video(VideoAction::encode_x264(None, None))
            .build(&media, &["yuv420p10le".into(), "yuv444p12le".into()])
            .unwrap();
        let pairs = |flag: &str| {
            plan.args
                .windows(2)
                .filter(|pair| pair[0] == flag)
                .map(|pair| pair[1].to_str().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(pairs("-map"), ["0:5", "0:9", "0:20"]);
        assert_eq!(
            plan.attachments
                .iter()
                .map(|attachment| (
                    attachment.input_index,
                    attachment.output_index,
                    attachment.input
                ))
                .collect::<Vec<_>>(),
            [
                (8, 3, AttachmentInput::CoverFrame),
                (15, 4, AttachmentInput::AttachmentStream),
                (12, 5, AttachmentInput::CoverFrame)
            ]
        );
        assert_eq!(plan.attachments[0].metadata, media.streams[0].tags);
        assert_eq!(plan.attachments[1].metadata, media.streams[3].tags);
        assert_eq!(plan.attachments[2].metadata["filename"], "cover-1.jpg");
        assert_eq!(plan.attachments[2].metadata["mimetype"], "image/jpeg");
        assert_eq!(
            pairs("-pix_fmt:1"),
            ["+yuv420p10le"],
            "first encoded video retains its mapped output index"
        );
        assert_eq!(pairs("-pix_fmt:2"), ["+yuv444p12le"]);
        assert_eq!(pairs("-max_interleave_delta"), ["0"]);

        let sample = plan.sample("12.5", "2", "sample".into(), true);
        let input = sample.args.iter().position(|arg| arg == "-i").unwrap();
        assert_eq!(&sample.args[input - 2..input], ["-ss", "12.5"]);
        assert_eq!(&sample.args[sample.args.len() - 2..], ["-t", "2"]);
        for flag in ["-map_metadata", "-map_chapters"] {
            let position = sample.args.iter().position(|arg| arg == flag).unwrap();
            assert_eq!(sample.args[position + 1], "-1");
        }
        assert_eq!(
            sample
                .attachments
                .iter()
                .map(|attachment| (
                    attachment.input_index,
                    attachment.output_index,
                    attachment.input
                ))
                .collect::<Vec<_>>(),
            [
                (8, 3, AttachmentInput::CoverFrame),
                (12, 4, AttachmentInput::CoverFrame)
            ]
        );
        assert!(
            plan.sample("12.5", "2", "sample".into(), false)
                .attachments
                .is_empty()
        );

        let mut video_and_attachment: MediaInfo = serde_json::from_str(
            r#"{"streams":[
                {"index":0,"codec_type":"video","pix_fmt":"yuv420p"},
                {"index":1,"codec_type":"attachment","tags":{"filename":"font.ttf","mimetype":"font/ttf"}}
            ]}"#,
        )
        .unwrap();
        video_and_attachment.pixel_formats = descriptors();
        let plan = TranscodeRequest::mkv("input", "output")
            .with_video(VideoAction::encode_x264(None, None))
            .build(&video_and_attachment, &["yuv420p".into()])
            .unwrap();
        assert!(
            !plan.args.iter().any(|arg| arg == "-max_interleave_delta"),
            "unbounded interleaving is only needed when a timed stream is copied"
        );

        // The Matroska-specific rewrite must not add extraction to MP4/MOV/WebM/TS.
        for container in [
            Container::Mp4,
            Container::Mov,
            Container::M4a,
            Container::ThreeGp,
            Container::ThreeG2,
            Container::F4v,
            Container::Ismv,
            Container::Psp,
            Container::Webm,
            Container::MpegTs,
            Container::M2ts,
            Container::Avi,
            Container::Flv,
            Container::Asf,
            Container::Wmv,
            Container::MpegPs,
            Container::Vob,
            Container::Ogg,
            Container::Ogv,
        ] {
            let plan = TranscodeRequest::new("input", "output")
                .with_container(container)
                .build(&media, &[])
                .unwrap();
            assert!(plan.attachments.is_empty());
            assert_eq!(
                plan.args.iter().filter(|arg| *arg == "-map").count(),
                media.streams.len()
            );
        }
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
            plan.args,
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
                plan.args
                    .windows(2)
                    .filter(|pair| pair[0] == "-vaapi_device" && pair[1] == "/dev/dri/custom")
                    .count(),
                devices
            );
            for option in ["-c:v", "-rc_mode:v", "-global_quality:v"] {
                assert_eq!(
                    plan.args.iter().filter(|arg| *arg == option).count(),
                    devices
                );
            }
            assert!(!plan.args.iter().any(|arg| arg == "-qp:v"));
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
                    plan.args
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
                assert_eq!(value("-c:2"), None);
                assert_eq!(plan.attachments[0].input_index, 3);
                assert_eq!(plan.attachments[0].output_index, 2);
                assert!(value("-pix_fmt:2").is_none());
                assert!(value("-hwaccel:3").is_none());
                let input = plan.args.iter().position(|a| a == "-i").unwrap();
                assert!(plan.args[..input].iter().any(|a| a == "-hwaccel:9"));
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
                plan.args
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
                plan.args
                    .windows(2)
                    .find(|p| p[0] == key)
                    .map(|p| p[1].to_str().unwrap())
            };
            assert_eq!(value("-pix_fmt:0"), Some(target));
            assert_eq!(value("-filter:0"), filter);
        }
    }
}
