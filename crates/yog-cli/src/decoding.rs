use std::ffi::OsString;

use yog_core::ffmpeg::decoding::DecodingBackend;

#[derive(Debug, clap::Args)]
#[group(id = "decoding", multiple = false)]
pub struct DecodingArgs {
    #[arg(
        long = "decode-vaapi",
        global = true,
        require_equals = true,
        value_name = "DEVICE"
    )]
    vaapi: Option<Option<OsString>>,
    #[arg(
        long = "decode-cuda",
        global = true,
        require_equals = true,
        value_name = "DEVICE"
    )]
    cuda: Option<Option<OsString>>,
    #[arg(
        long = "decode-qsv",
        global = true,
        require_equals = true,
        value_name = "DEVICE"
    )]
    qsv: Option<Option<OsString>>,
}

impl TryFrom<DecodingArgs> for DecodingBackend {
    type Error = clap::Error;

    fn try_from(args: DecodingArgs) -> Result<Self, Self::Error> {
        Ok(match (args.vaapi, args.cuda, args.qsv) {
            (None, None, None) => Self::Software,
            (Some(device), None, None) => Self::Vaapi(device),
            (None, Some(device), None) => Self::Cuda(device),
            (None, None, Some(device)) => Self::Qsv(device),
            _ => {
                return Err(clap::Error::raw(
                    clap::error::ErrorKind::ArgumentConflict,
                    "--decode-vaapi, --decode-cuda and --decode-qsv are mutually exclusive",
                ));
            }
        })
    }
}
