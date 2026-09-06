use std::ffi::OsString;

/// FFmpeg arguments are separate from ffprobe arguments.
pub(super) enum Arg<'a> {
    /// -hide_banner
    HideBanner,
    /// -h encoder=
    EncoderHelp(&'a str),
}

impl Arg<'_> {
    pub fn append_to(self, args: &mut Vec<OsString>) {
        match self {
            Self::HideBanner => args.push("-hide_banner".into()),
            Self::EncoderHelp(encoder) => {
                args.push("-h".into());
                args.push(format!("encoder={encoder}").into());
            }
        }
    }
}
