use std::ffi::OsString;

/// FFmpeg arguments are separate from ffprobe arguments.
pub(super) enum Arg<'a> {
    /// -hide_banner
    HideBanner,
    /// -nostdin
    NoStdin,
    /// -nostats
    NoStats,
    /// -progress pipe:1
    ProgressStdout,
    /// -h encoder=
    EncoderHelp(&'a str),
}

impl Arg<'_> {
    pub fn append_to(self, args: &mut Vec<OsString>) {
        match self {
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
