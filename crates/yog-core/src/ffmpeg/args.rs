use std::ffi::{OsStr, OsString};
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
    /// -hwaccel {method}
    Hwaccel(&'a str),
    /// -hwaccel_device {device}
    HwaccelDevice(&'a OsStr),
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
    /// -c copy
    CopyAll,
    /// -c:v {name}
    VideoCodec(&'a str),
    /// -c:{output_index} copy
    CopyStream(usize),
    /// {option}:v {value}
    Video {
        option: VideoOption,
        value: OsString,
    },
    /// -f {format}
    Format(&'a str),
}

pub(super) trait ArgsExt {
    fn add(&mut self, arg: Arg<'_>);
}

impl ArgsExt for Vec<OsString> {
    fn add(&mut self, arg: Arg<'_>) {
        let (flag, value): (OsString, Option<OsString>) = match arg {
            Arg::Overwrite(overwrite) => (if overwrite { "-y" } else { "-n" }.into(), None),
            Arg::Hwaccel(method) => ("-hwaccel".into(), Some(method.into())),
            Arg::HwaccelDevice(device) => ("-hwaccel_device".into(), Some(device.to_owned())),
            Arg::Input(path) => ("-i".into(), Some(path.as_os_str().to_owned())),
            Arg::Output(path) => (path.as_os_str().to_owned(), None),
            Arg::VaapiDevice(path) => ("-vaapi_device".into(), Some(path.as_os_str().to_owned())),
            Arg::Map(index) => ("-map".into(), Some(format!("0:{index}").into())),
            Arg::MapMetadata => ("-map_metadata".into(), Some("0".into())),
            Arg::MapChapters => ("-map_chapters".into(), Some("0".into())),
            Arg::CopyAll => ("-c".into(), Some("copy".into())),
            Arg::VideoCodec(name) => ("-c:v".into(), Some(name.into())),
            Arg::CopyStream(index) => (format!("-c:{index}").into(), Some("copy".into())),
            Arg::Video { option, value } => (format!("{}:v", option.flag()).into(), Some(value)),
            Arg::Format(format) => ("-f".into(), Some(format.into())),
            Arg::HideBanner => ("-hide_banner".into(), None),
            Arg::NoStdin => ("-nostdin".into(), None),
            Arg::NoStats => ("-nostats".into(), None),
            Arg::ProgressStdout => ("-progress".into(), Some("pipe:1".into())),
            Arg::EncoderHelp(encoder) => ("-h".into(), Some(format!("encoder={encoder}").into())),
        };
        self.push(flag);
        self.extend(value);
    }
}

impl<'a> Extend<Arg<'a>> for Vec<OsString> {
    fn extend<T: IntoIterator<Item = Arg<'a>>>(&mut self, args: T) {
        for arg in args {
            self.add(arg);
        }
    }
}
