use super::args::{Arg, ArgsExt, VideoOption};
use std::{
    borrow::Cow, ffi::OsString, fmt::Display, num::NonZeroU64, ops::RangeInclusive, path::PathBuf,
    str::FromStr,
};

#[cfg(feature = "clap")]
mod cli;
#[cfg(test)]
mod tests;

pub const DEFAULT_VAAPI_DEVICE: &str = "/dev/dri/renderD128";

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
}

impl Display for VideoCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoCodec::H264 => write!(f, "H.264"),
            VideoCodec::Hevc => write!(f, "HEVC"),
            VideoCodec::Av1 => write!(f, "AV1"),
        }
    }
}

impl FromStr for VideoCodec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "h264" => Ok(Self::H264),
            "hevc" => Ok(Self::Hevc),
            "av1" => Ok(Self::Av1),
            _ => Err(format!("unknown video codec {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RateControl {
    Quality(u8),
    Bitrate(NonZeroU64),
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Preset {
    Ultrafast,
    Superfast,
    Veryfast,
    Faster,
    Fast,
    Medium,
    Slow,
    Slower,
    Veryslow,
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ultrafast => "ultrafast",
            Self::Superfast => "superfast",
            Self::Veryfast => "veryfast",
            Self::Faster => "faster",
            Self::Fast => "fast",
            Self::Medium => "medium",
            Self::Slow => "slow",
            Self::Slower => "slower",
            Self::Veryslow => "veryslow",
        }
    }
}

impl FromStr for Preset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ultrafast" => Ok(Self::Ultrafast),
            "superfast" => Ok(Self::Superfast),
            "veryfast" => Ok(Self::Veryfast),
            "faster" => Ok(Self::Faster),
            "fast" => Ok(Self::Fast),
            "medium" => Ok(Self::Medium),
            "slow" => Ok(Self::Slow),
            "slower" => Ok(Self::Slower),
            "veryslow" => Ok(Self::Veryslow),
            _ => Err(format!("unknown x264/x265 preset {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum QsvPreset {
    Veryfast,
    Faster,
    Fast,
    Medium,
    Slow,
    Slower,
    Veryslow,
}

impl QsvPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Veryfast => "veryfast",
            Self::Faster => "faster",
            Self::Fast => "fast",
            Self::Medium => "medium",
            Self::Slow => "slow",
            Self::Slower => "slower",
            Self::Veryslow => "veryslow",
        }
    }
}

impl FromStr for QsvPreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "veryfast" => Ok(Self::Veryfast),
            "faster" => Ok(Self::Faster),
            "fast" => Ok(Self::Fast),
            "medium" => Ok(Self::Medium),
            "slow" => Ok(Self::Slow),
            "slower" => Ok(Self::Slower),
            "veryslow" => Ok(Self::Veryslow),
            _ => Err(format!("unknown QSV preset {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum NvencPreset {
    P1,
    P2,
    P3,
    P4,
    P5,
    P6,
    P7,
}

impl NvencPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::P1 => "p1",
            Self::P2 => "p2",
            Self::P3 => "p3",
            Self::P4 => "p4",
            Self::P5 => "p5",
            Self::P6 => "p6",
            Self::P7 => "p7",
        }
    }
}

impl FromStr for NvencPreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "p1" => Ok(Self::P1),
            "p2" => Ok(Self::P2),
            "p3" => Ok(Self::P3),
            "p4" => Ok(Self::P4),
            "p5" => Ok(Self::P5),
            "p6" => Ok(Self::P6),
            "p7" => Ok(Self::P7),
            _ => Err(format!("unknown NVENC preset {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum NvencMultipass {
    Disabled,
    #[cfg_attr(feature = "clap", value(name = "qres"))]
    QuarterResolution,
    #[cfg_attr(feature = "clap", value(name = "fullres"))]
    FullResolution,
}

impl FromStr for NvencMultipass {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "qres" => Ok(Self::QuarterResolution),
            "fullres" => Ok(Self::FullResolution),
            _ => Err(format!("unknown NVENC multipass mode {value}")),
        }
    }
}

impl NvencMultipass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::QuarterResolution => "qres",
            Self::FullResolution => "fullres",
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "clap", derive(clap::Subcommand))]
pub enum VideoEncoding {
    #[cfg_attr(feature = "clap", command(long_flag = "encode-x264"))]
    X264 {
        #[cfg_attr(feature = "clap", command(flatten))]
        rate: Option<RateControl>,
        #[cfg_attr(feature = "clap", arg(long, short))]
        preset: Option<Preset>,
    },
    #[cfg_attr(feature = "clap", command(long_flag = "encode-x265"))]
    X265 {
        #[cfg_attr(feature = "clap", command(flatten))]
        rate: Option<RateControl>,
        #[cfg_attr(feature = "clap", arg(long, short))]
        preset: Option<Preset>,
    },
    #[cfg_attr(feature = "clap", command(long_flag = "encode-svt-av1"))]
    SvtAv1 {
        #[cfg_attr(feature = "clap", command(flatten))]
        rate: Option<RateControl>,
        #[cfg_attr(feature = "clap", arg(long, short, value_parser = clap::value_parser!(u8).range(0..=13)))]
        /// [range: 0-13]
        preset: Option<u8>,
    },
    #[cfg_attr(feature = "clap", command(long_flag = "encode-aom-av1"))]
    AomAv1 {
        #[cfg_attr(feature = "clap", command(flatten))]
        rate: Option<RateControl>,
        #[cfg_attr(feature = "clap", arg(long, short = 'u', value_parser = clap::value_parser!(u8).range(0..=8)))]
        /// [range: 0-8]
        cpu_used: Option<u8>,
    },
    #[cfg_attr(feature = "clap", command(long_flag = "encode-rav1e"))]
    Rav1e {
        #[cfg_attr(feature = "clap", command(flatten))]
        rate: Option<RateControl>,
        #[cfg_attr(feature = "clap", arg(long, short, value_parser = clap::value_parser!(u8).range(0..=10)))]
        /// [range: 0-10]
        speed: Option<u8>,
    },
    #[cfg_attr(feature = "clap", command(long_flag = "encode-nvenc"))]
    Nvenc {
        codec: VideoCodec,
        #[cfg_attr(feature = "clap", command(flatten))]
        rate: Option<RateControl>,
        #[cfg_attr(feature = "clap", arg(long, short))]
        preset: Option<NvencPreset>,
        #[cfg_attr(feature = "clap", arg(long, short))]
        multipass: Option<NvencMultipass>,
    },
    #[cfg_attr(feature = "clap", command(long_flag = "encode-qsv"))]
    Qsv {
        codec: VideoCodec,
        #[cfg_attr(feature = "clap", command(flatten))]
        rate: Option<RateControl>,
        #[cfg_attr(feature = "clap", arg(long, short))]
        preset: Option<QsvPreset>,
    },
    #[cfg_attr(feature = "clap", command(long_flag = "encode-vaapi"))]
    Vaapi {
        codec: VideoCodec,
        #[cfg_attr(feature = "clap", command(flatten))]
        rate: Option<RateControl>,
        #[cfg_attr(
            feature = "clap",
            arg(long, short, default_value = DEFAULT_VAAPI_DEVICE)
        )]
        device: PathBuf,
    },
}

impl Default for VideoEncoding {
    fn default() -> Self {
        Self::X264 {
            rate: None,
            preset: None,
        }
    }
}

impl FromStr for VideoEncoding {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (name, device) = match value.split_once('=') {
            Some((name, device)) => (name, Some(device)),
            None => (value, None),
        };

        if let Some((codec, backend)) = name.split_once('_') {
            let codec = codec.parse()?;
            return match backend {
                "nvenc" if device.is_none() => Ok(Self::Nvenc {
                    codec,
                    rate: None,
                    preset: None,
                    multipass: None,
                }),
                "qsv" if device.is_none() => Ok(Self::Qsv {
                    codec,
                    rate: None,
                    preset: None,
                }),
                "vaapi" => Ok(Self::Vaapi {
                    codec,
                    rate: None,
                    device: device
                        .filter(|device| !device.is_empty())
                        .unwrap_or(DEFAULT_VAAPI_DEVICE)
                        .into(),
                }),
                "nvenc" | "qsv" => Err(format!("{backend} encoding does not accept a device")),
                _ => Err(format!("unknown encoder {name}")),
            };
        }

        if device.is_some() {
            return Err(format!("{name} encoding does not accept a device"));
        }

        match name {
            "libx264" => Ok(Self::X264 {
                rate: None,
                preset: None,
            }),
            "libx265" => Ok(Self::X265 {
                rate: None,
                preset: None,
            }),
            "libsvtav1" => Ok(Self::SvtAv1 {
                rate: None,
                preset: None,
            }),
            "libaom-av1" => Ok(Self::AomAv1 {
                rate: None,
                cpu_used: None,
            }),
            "librav1e" => Ok(Self::Rav1e {
                rate: None,
                speed: None,
            }),
            _ => Err(format!("unknown encoder {name}")),
        }
    }
}

impl Display for VideoEncoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoEncoding::X264 { .. } => write!(f, "x264"),
            VideoEncoding::X265 { .. } => write!(f, "x265"),
            VideoEncoding::SvtAv1 { .. } => write!(f, "SVT-AV1"),
            VideoEncoding::AomAv1 { .. } => write!(f, "AOM-AV1"),
            VideoEncoding::Rav1e { .. } => write!(f, "rav1e"),
            VideoEncoding::Nvenc { codec, .. } => write!(f, "{} NVENC", codec),
            VideoEncoding::Qsv { codec, .. } => write!(f, "{} QSV", codec),
            VideoEncoding::Vaapi { codec, .. } => write!(f, "{} VAAPI", codec),
        }
    }
}

impl VideoEncoding {
    pub fn preset(&self) -> Option<Cow<'static, str>> {
        match self {
            VideoEncoding::X264 { preset, .. } | VideoEncoding::X265 { preset, .. } => {
                preset.map(|value| value.as_str().into())
            }
            VideoEncoding::Qsv { preset, .. } => preset.map(|value| value.as_str().into()),
            VideoEncoding::Nvenc { preset, .. } => preset.map(|value| value.as_str().into()),
            VideoEncoding::SvtAv1 { preset, .. } => preset.map(|value| value.to_string().into()),
            VideoEncoding::AomAv1 { cpu_used, .. } => {
                cpu_used.map(|value| value.to_string().into())
            }
            VideoEncoding::Rav1e { speed, .. } => speed.map(|value| value.to_string().into()),
            VideoEncoding::Vaapi { .. } => None,
        }
    }

    pub fn multipass(&self) -> Option<&'static str> {
        match self {
            VideoEncoding::Nvenc { multipass, .. } => multipass.map(|value| value.as_str()),
            _ => None,
        }
    }

    pub fn try_set_preset(&mut self, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Ok(());
        }
        let number = || value.parse::<u8>().map_err(|error| error.to_string());

        match self {
            Self::X264 { preset, .. } | Self::X265 { preset, .. } => {
                *preset = Some(value.parse()?);
            }
            Self::SvtAv1 { preset, .. } => *preset = Some(number()?),
            Self::AomAv1 { cpu_used, .. } => *cpu_used = Some(number()?),
            Self::Rav1e { speed, .. } => *speed = Some(number()?),
            Self::Nvenc { preset, .. } => *preset = Some(value.parse()?),
            Self::Qsv { preset, .. } => *preset = Some(value.parse()?),
            Self::Vaapi { .. } => return Err("VAAPI does not support preset".to_owned()),
        }

        Ok(())
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::X264 { .. } => "libx264",
            Self::X265 { .. } => "libx265",
            Self::SvtAv1 { .. } => "libsvtav1",
            Self::AomAv1 { .. } => "libaom-av1",
            Self::Rav1e { .. } => "librav1e",
            Self::Nvenc { codec, .. } => match codec {
                VideoCodec::H264 => "h264_nvenc",
                VideoCodec::Hevc => "hevc_nvenc",
                VideoCodec::Av1 => "av1_nvenc",
            },
            Self::Qsv { codec, .. } => match codec {
                VideoCodec::H264 => "h264_qsv",
                VideoCodec::Hevc => "hevc_qsv",
                VideoCodec::Av1 => "av1_qsv",
            },
            Self::Vaapi { codec, .. } => match codec {
                VideoCodec::H264 => "h264_vaapi",
                VideoCodec::Hevc => "hevc_vaapi",
                VideoCodec::Av1 => "av1_vaapi",
            },
        }
    }

    pub fn quality_range(&self) -> RangeInclusive<u8> {
        match &self {
            Self::X264 { .. } | Self::X265 { .. } => 0..=51,
            Self::SvtAv1 { .. } | Self::AomAv1 { .. } => 0..=63,
            Self::Rav1e { .. } => 0..=255,
            Self::Nvenc { codec, .. } => match codec {
                VideoCodec::H264 | VideoCodec::Hevc => 0..=51,
                VideoCodec::Av1 => 0..=63,
            },
            Self::Qsv { .. } => 1..=51,
            Self::Vaapi {
                codec: VideoCodec::H264,
                ..
            }
            | Self::Vaapi {
                codec: VideoCodec::Hevc,
                ..
            } => 0..=52,
            Self::Vaapi {
                codec: VideoCodec::Av1,
                ..
            } => 0..=255,
        }
    }

    pub fn quality_parameter(&self) -> &'static str {
        match self.quality_option() {
            VideoOption::Crf => "CRF",
            VideoOption::Cq => "CQ",
            VideoOption::Qp => "QP",
            VideoOption::GlobalQuality => "global_quality",
            _ => unreachable!("quality controls only use quality-related video options"),
        }
    }

    pub fn rate(&self) -> Option<RateControl> {
        match self {
            Self::X264 { rate, .. }
            | Self::X265 { rate, .. }
            | Self::SvtAv1 { rate, .. }
            | Self::AomAv1 { rate, .. }
            | Self::Rav1e { rate, .. }
            | Self::Nvenc { rate, .. }
            | Self::Qsv { rate, .. }
            | Self::Vaapi { rate, .. } => *rate,
        }
    }

    pub fn set_quality(&mut self, quality: u8) {
        self.set_rate(Some(RateControl::Quality(quality)));
    }

    pub fn set_rate(&mut self, value: Option<RateControl>) {
        let rate = match self {
            Self::X264 { rate, .. }
            | Self::X265 { rate, .. }
            | Self::SvtAv1 { rate, .. }
            | Self::AomAv1 { rate, .. }
            | Self::Rav1e { rate, .. }
            | Self::Nvenc { rate, .. }
            | Self::Qsv { rate, .. }
            | Self::Vaapi { rate, .. } => rate,
        };
        *rate = value;
    }

    fn quality_option(&self) -> VideoOption {
        match self {
            Self::Nvenc { .. } => VideoOption::Cq,
            Self::Qsv { .. }
            | Self::Vaapi {
                codec: VideoCodec::Av1,
                ..
            } => VideoOption::GlobalQuality,
            Self::Vaapi { .. } | Self::Rav1e { .. } => VideoOption::Qp,
            Self::X264 { .. } | Self::X265 { .. } | Self::SvtAv1 { .. } | Self::AomAv1 { .. } => {
                VideoOption::Crf
            }
        }
    }

    pub(super) fn append_options(&self, args: &mut Vec<OsString>) {
        let rate = self.rate();
        let mut append = |option, value: OsString| args.add(Arg::Video { option, value });
        match rate {
            Some(RateControl::Bitrate(bitrate)) => {
                append(VideoOption::Bitrate, bitrate.to_string().into())
            }
            Some(RateControl::Quality(quality)) => {
                match self {
                    Self::Nvenc { .. } => {
                        append(VideoOption::RateControl, "vbr".into());
                        append(VideoOption::Bitrate, "0".into());
                    }
                    Self::Vaapi {
                        codec: VideoCodec::Av1,
                        ..
                    } => {
                        append(VideoOption::RateControlMode, "CQP".into());
                    }
                    Self::Vaapi { .. } => {
                        append(VideoOption::RateControlMode, "CQP".into());
                    }
                    Self::AomAv1 { .. } => {
                        append(VideoOption::Bitrate, "0".into());
                    }
                    _ => {}
                }
                append(self.quality_option(), quality.to_string().into());
            }
            None => {}
        }
        match self {
            Self::X264 { preset, .. } | Self::X265 { preset, .. } => {
                if let Some(preset) = preset {
                    append(VideoOption::Preset, preset.as_str().into());
                }
            }
            Self::Qsv { preset, .. } => {
                if let Some(preset) = preset {
                    append(VideoOption::Preset, preset.as_str().into());
                }
            }
            Self::SvtAv1 { preset, .. } => {
                if let Some(preset) = preset {
                    append(VideoOption::Preset, preset.to_string().into());
                }
            }
            Self::AomAv1 { cpu_used, .. } => {
                if let Some(value) = cpu_used {
                    append(VideoOption::CpuUsed, value.to_string().into());
                }
            }
            Self::Rav1e { speed, .. } => {
                if let Some(value) = speed {
                    append(VideoOption::Speed, value.to_string().into());
                }
            }
            Self::Nvenc {
                preset, multipass, ..
            } => {
                if let Some(preset) = preset {
                    append(VideoOption::Preset, preset.as_str().into());
                }
                if let Some(value) = multipass {
                    append(VideoOption::Multipass, value.as_str().into());
                }
            }
            Self::Vaapi { .. } => {}
        }
    }
}
