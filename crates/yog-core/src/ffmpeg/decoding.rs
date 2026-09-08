use super::args::Arg;
use std::ffi::OsString;

#[derive(Debug, Clone, Default)]
pub enum DecodingBackend {
    #[default]
    Software,
    Vaapi(Option<OsString>),
    Cuda(Option<OsString>),
    Qsv(Option<OsString>),
}

impl DecodingBackend {
    pub(super) fn args(&self) -> impl Iterator<Item = Arg<'_>> {
        let (method, device) = match self {
            Self::Software => ("none", None),
            Self::Vaapi(device) => ("vaapi", device.as_deref()),
            Self::Cuda(device) => ("cuda", device.as_deref()),
            Self::Qsv(device) => ("qsv", device.as_deref()),
        };
        std::iter::once(Arg::Hwaccel(method)).chain(device.map(Arg::HwaccelDevice))
    }
}
