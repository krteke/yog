use super::args::{Arg, VideoOption};
use std::{ffi::OsString, num::NonZeroU64, path::PathBuf};

#[derive(Debug, Clone, Copy)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
}

#[derive(Debug, Clone, Copy)]
pub enum RateControl {
    Quality(u8),
    Bitrate(NonZeroU64),
}

#[derive(Debug, Clone, Copy)]
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
    pub(super) fn value(self) -> &'static str {
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

#[derive(Debug, Clone, Copy)]
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
    pub(super) fn value(self) -> &'static str {
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

#[derive(Debug, Clone, Copy)]
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
    pub(super) fn value(self) -> &'static str {
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

#[derive(Debug, Clone, Copy)]
pub enum NvencMultipass {
    Disabled,
    QuarterResolution,
    FullResolution,
}

impl NvencMultipass {
    pub(super) fn value(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::QuarterResolution => "qres",
            Self::FullResolution => "fullres",
        }
    }
}

#[derive(Debug, Clone)]
pub enum VideoEncoding {
    X264 {
        rate: Option<RateControl>,
        preset: Option<Preset>,
    },
    X265 {
        rate: Option<RateControl>,
        preset: Option<Preset>,
    },
    SvtAv1 {
        rate: Option<RateControl>,
        preset: Option<u8>,
    },
    AomAv1 {
        rate: Option<RateControl>,
        cpu_used: Option<u8>,
    },
    Rav1e {
        rate: Option<RateControl>,
        speed: Option<u8>,
    },
    Nvenc {
        codec: VideoCodec,
        rate: Option<RateControl>,
        preset: Option<NvencPreset>,
        multipass: Option<NvencMultipass>,
    },
    Qsv {
        codec: VideoCodec,
        rate: Option<RateControl>,
        preset: Option<QsvPreset>,
    },
    Vaapi {
        codec: VideoCodec,
        rate: Option<RateControl>,
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

impl VideoEncoding {
    pub(super) fn name(&self) -> &'static str {
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

    pub(super) fn append_options(&self, ordinal: usize, args: &mut Vec<OsString>) {
        let rate = match self {
            Self::X264 { rate, .. }
            | Self::X265 { rate, .. }
            | Self::SvtAv1 { rate, .. }
            | Self::AomAv1 { rate, .. }
            | Self::Rav1e { rate, .. }
            | Self::Nvenc { rate, .. }
            | Self::Qsv { rate, .. }
            | Self::Vaapi { rate, .. } => *rate,
        };
        let mut append = |option, value: OsString| {
            Arg::Video {
                ordinal,
                option,
                value,
            }
            .append_to(args)
        };
        match rate {
            Some(RateControl::Bitrate(value)) => {
                append(VideoOption::Bitrate, value.to_string().into())
            }
            Some(RateControl::Quality(value)) => {
                let option = match self {
                    Self::Nvenc { .. } => {
                        append(VideoOption::RateControl, "vbr".into());
                        append(VideoOption::Bitrate, "0".into());
                        VideoOption::Cq
                    }
                    Self::Qsv { .. } => VideoOption::GlobalQuality,
                    Self::Vaapi { .. } => {
                        append(VideoOption::RateControlMode, "CQP".into());
                        VideoOption::Qp
                    }
                    Self::Rav1e { .. } => VideoOption::Qp,
                    Self::AomAv1 { .. } => {
                        append(VideoOption::Bitrate, "0".into());
                        VideoOption::Crf
                    }
                    _ => VideoOption::Crf,
                };
                append(option, value.to_string().into());
            }
            None => {}
        }
        match self {
            Self::X264 { preset, .. } | Self::X265 { preset, .. } => {
                if let Some(preset) = preset {
                    append(VideoOption::Preset, preset.value().into());
                }
            }
            Self::Qsv { preset, .. } => {
                if let Some(preset) = preset {
                    append(VideoOption::Preset, preset.value().into());
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
                    append(VideoOption::Preset, preset.value().into());
                }
                if let Some(value) = multipass {
                    append(VideoOption::Multipass, value.value().into());
                }
            }
            Self::Vaapi { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quality_controls_are_encoder_specific_and_bitrate_does_not_add_limits() {
        let quality = Some(RateControl::Quality(30));
        let cases = [
            (
                VideoEncoding::X264 {
                    rate: quality,
                    preset: None,
                },
                vec!["-crf:v:2", "30"],
            ),
            (
                VideoEncoding::X265 {
                    rate: quality,
                    preset: None,
                },
                vec!["-crf:v:2", "30"],
            ),
            (
                VideoEncoding::SvtAv1 {
                    rate: quality,
                    preset: None,
                },
                vec!["-crf:v:2", "30"],
            ),
            (
                VideoEncoding::AomAv1 {
                    rate: quality,
                    cpu_used: None,
                },
                vec!["-b:v:2", "0", "-crf:v:2", "30"],
            ),
            (
                VideoEncoding::Rav1e {
                    rate: quality,
                    speed: None,
                },
                vec!["-qp:v:2", "30"],
            ),
            (
                VideoEncoding::Qsv {
                    codec: VideoCodec::H264,
                    rate: quality,
                    preset: None,
                },
                vec!["-global_quality:v:2", "30"],
            ),
            (
                VideoEncoding::Vaapi {
                    codec: VideoCodec::Hevc,
                    rate: quality,
                    device: "/dev/dri/renderD128".into(),
                },
                vec!["-rc_mode:v:2", "CQP", "-qp:v:2", "30"],
            ),
            (
                VideoEncoding::Nvenc {
                    codec: VideoCodec::Av1,
                    rate: Some(RateControl::Bitrate(NonZeroU64::new(4_000_000).unwrap())),
                    preset: None,
                    multipass: None,
                },
                vec!["-b:v:2", "4000000"],
            ),
        ];
        for (encoding, expected) in cases {
            let mut args = Vec::new();
            encoding.append_options(2, &mut args);
            assert_eq!(
                args,
                expected.into_iter().map(OsString::from).collect::<Vec<_>>(),
                "{encoding:?}"
            );
        }
    }
}
