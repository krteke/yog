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
    /// -v {level}
    LogLevel(&'a str),
    /// -progress pipe:1
    ProgressStdout,
    /// -h encoder={encoder}
    EncoderHelp(&'a str),
    /// -y/-n
    Overwrite(bool),
    /// -i {path}
    Input(&'a Path),
    /// -ss {timestamp}
    Seek(&'a str),
    /// -dump_attachment:{input_index} {path}
    DumpAttachment(usize, &'a Path),
    /// -hwaccel:{input_index} {method}
    Hwaccel(usize, &'a str),
    /// -hwaccel_device:{input_index} {device}
    HwaccelDevice(usize, &'a OsStr),
    /// -hwaccel_output_format:{input_index} {format}
    HwaccelOutputFormat(usize, &'a str),
    /// -pix_fmt:{output_index} +{format}
    PixelFormat(usize, &'a str),
    /// -filter:{output_index} {filter}
    Filter(usize, &'a str),
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
    /// -t {duration}
    Duration(&'a str),
    /// -frames:v 1
    OneVideoFrame,
    /// pipe:1
    Stdout,
    /// -attach {path}
    Attach(&'a Path),
    /// -max_interleave_delta {ms}
    MaxInterleaveDelta(u64),
    /// -metadata:s:{output_index} {key}={value}
    StreamMetadata(usize, &'a str, &'a str),
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
            Arg::Hwaccel(index, method) => {
                (format!("-hwaccel:{index}").into(), Some(method.into()))
            }
            Arg::HwaccelDevice(index, device) => (
                format!("-hwaccel_device:{index}").into(),
                Some(device.to_owned()),
            ),
            Arg::HwaccelOutputFormat(index, format) => (
                format!("-hwaccel_output_format:{index}").into(),
                Some(format.into()),
            ),
            Arg::PixelFormat(index, format) => (
                format!("-pix_fmt:{index}").into(),
                Some(format!("+{format}").into()),
            ),
            Arg::Filter(index, filter) => (format!("-filter:{index}").into(), Some(filter.into())),
            Arg::Input(path) => ("-i".into(), Some(path.as_os_str().to_owned())),
            Arg::Seek(timestamp) => ("-ss".into(), Some(timestamp.into())),
            Arg::DumpAttachment(index, path) => (
                format!("-dump_attachment:{index}").into(),
                Some(path.as_os_str().to_owned()),
            ),
            Arg::Output(path) => (path.as_os_str().to_owned(), None),
            Arg::VaapiDevice(path) => ("-vaapi_device".into(), Some(path.as_os_str().to_owned())),
            Arg::Map(index) => ("-map".into(), Some(format!("0:{index}").into())),
            Arg::MapMetadata => ("-map_metadata".into(), Some("0".into())),
            Arg::MapChapters => ("-map_chapters".into(), Some("0".into())),
            Arg::CopyAll => ("-c".into(), Some("copy".into())),
            Arg::VideoCodec(name) => ("-c:v".into(), Some(name.into())),
            Arg::CopyStream(index) => (format!("-c:{index}").into(), Some("copy".into())),
            Arg::Duration(duration) => ("-t".into(), Some(duration.into())),
            Arg::OneVideoFrame => ("-frames:v".into(), Some("1".into())),
            Arg::Stdout => ("pipe:1".into(), None),
            Arg::Attach(path) => ("-attach".into(), Some(path.as_os_str().to_owned())),
            Arg::MaxInterleaveDelta(microseconds) => (
                "-max_interleave_delta".into(),
                Some(microseconds.to_string().into()),
            ),
            Arg::StreamMetadata(index, key, value) => (
                format!("-metadata:s:{index}").into(),
                Some(format!("{key}={value}").into()),
            ),
            Arg::Video { option, value } => (format!("{}:v", option.flag()).into(), Some(value)),
            Arg::Format(format) => ("-f".into(), Some(format.into())),
            Arg::HideBanner => ("-hide_banner".into(), None),
            Arg::NoStdin => ("-nostdin".into(), None),
            Arg::NoStats => ("-nostats".into(), None),
            Arg::LogLevel(level) => ("-v".into(), Some(level.into())),
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
