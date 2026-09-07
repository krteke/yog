use std::ffi::OsString;
use std::path::Path;

pub(super) enum VideoOption {
    /// -b
    Bitrate,
    /// -crf
    Crf,
    /// -cq
    Cq,
    /// -qp
    Qp,
    /// -global_quality
    GlobalQuality,
    /// -rc
    RateControl,
    /// -rc_mode
    RateControlMode,
    /// -preset
    Preset,
    /// -cpu-used
    CpuUsed,
    /// -speed
    Speed,
    /// -multipass
    Multipass,
}

impl VideoOption {
    fn flag(self) -> &'static str {
        match self {
            Self::Bitrate => "-b",
            Self::Crf => "-crf",
            Self::Cq => "-cq",
            Self::Qp => "-qp",
            Self::GlobalQuality => "-global_quality",
            Self::RateControl => "-rc",
            Self::RateControlMode => "-rc_mode",
            Self::Preset => "-preset",
            Self::CpuUsed => "-cpu-used",
            Self::Speed => "-speed",
            Self::Multipass => "-multipass",
        }
    }
}

pub(super) enum Arg<'a> {
    /// -hide_banner
    HideBanner,
    /// -nostdin
    NoStdin,
    /// -nostats
    NoStats,
    /// -progress pipe:1
    ProgressStdout,
    /// -h encoder={encoder}
    EncoderHelp(&'a str),
    /// -y/-n
    Overwrite(bool),
    /// -i {path}
    Input(&'a Path),
    /// {path}
    Output(&'a Path),
    /// -vaapi_device
    VaapiDevice(&'a Path),
    /// -map 0:{index}
    Map(usize),
    /// -map_metadata 0
    MapMetadata,
    /// -map_chapters 0
    MapChapters,
    /// -c:{index} {name}
    Codec { index: usize, name: &'a str },
    /// {option}:v:{ordinal} {value}
    Video {
        ordinal: usize,
        option: VideoOption,
        value: OsString,
    },
    /// -f {format}
    Format(&'a str),
}

impl Arg<'_> {
    pub fn append_to(self, args: &mut Vec<OsString>) {
        match self {
            Self::Overwrite(overwrite) => args.push(if overwrite { "-y" } else { "-n" }.into()),
            Self::Input(path) => {
                args.push("-i".into());
                args.push(path.as_os_str().to_owned());
            }
            Self::Output(path) => {
                args.push(path.as_os_str().to_owned());
            }
            Self::VaapiDevice(path) => {
                args.extend(["-vaapi_device".into(), path.as_os_str().to_owned()])
            }
            Self::Map(index) => args.extend(["-map".into(), format!("0:{index}").into()]),
            Self::MapMetadata => args.extend(["-map_metadata".into(), "0".into()]),
            Self::MapChapters => args.extend(["-map_chapters".into(), "0".into()]),
            Self::Codec { index, name } => args.extend([format!("-c:{index}").into(), name.into()]),
            Self::Video {
                ordinal,
                option,
                value,
            } => args.extend([format!("{}:v:{ordinal}", option.flag()).into(), value]),
            Self::Format(format) => args.extend(["-f".into(), format.into()]),
            Self::HideBanner => args.push("-hide_banner".into()),
            Self::NoStdin => args.push("-nostdin".into()),
            Self::NoStats => args.push("-nostats".into()),
            Self::ProgressStdout => args.extend(["-progress".into(), "pipe:1".into()]),
            Self::EncoderHelp(encoder) => {
                args.push("-h".into());
                args.push(format!("encoder={encoder}").into());
            }
        }
    }
}
