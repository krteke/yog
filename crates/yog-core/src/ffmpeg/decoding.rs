use std::{ffi::OsString, fmt::Display};

use crate::{
    ffmpeg::{
        args::{Arg, ArgsExt},
        encoding::VideoEncoding,
        pixel_format,
    },
    ffprobe::{
        pixel_format::{Chroma, VideoSamples},
        types::PixelFormat,
    },
};

pub(super) struct FramePlan {
    pub input_args: Vec<OsString>,
    pub output_args: Vec<OsString>,
}

#[derive(Debug, Clone, Default)]
pub enum DecodingBackend {
    #[default]
    Software,
    Vaapi(Option<OsString>),
    Cuda(Option<OsString>),
    Qsv(Option<OsString>),
}

impl Display for DecodingBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let device = match self {
            Self::Software => return f.write_str("software"),
            Self::Vaapi(device) => {
                f.write_str("vaapi")?;
                device
            }
            Self::Cuda(device) => {
                f.write_str("cuda")?;
                device
            }
            Self::Qsv(device) => {
                f.write_str("qsv")?;
                device
            }
        };

        if let Some(device) = device {
            f.write_fmt(format_args!(":{}", device.display()))?;
        }

        Ok(())
    }
}

impl DecodingBackend {
    pub(super) fn plan(
        &self,
        encoding: &VideoEncoding,
        source: &PixelFormat,
        input_index: usize,
        output_index: usize,
        target: &str,
    ) -> FramePlan {
        let mut input_args = Vec::new();
        let mut output_args = Vec::new();

        let (method, device) = match self {
            DecodingBackend::Software => ("none", None),
            DecodingBackend::Vaapi(device) => ("vaapi", device.as_deref()),
            DecodingBackend::Cuda(device) => ("cuda", device.as_deref()),
            DecodingBackend::Qsv(device) => ("qsv", device.as_deref()),
        };
        let device = match (self, encoding) {
            (DecodingBackend::Vaapi(None), VideoEncoding::Vaapi { device, .. }) => {
                Some(device.as_os_str())
            }
            _ => device,
        };

        input_args.add(Arg::Hwaccel(input_index, method));
        if let Some(device) = device {
            input_args.add(Arg::HwaccelDevice(input_index, device));
        }
        if method != "none" {
            input_args.add(Arg::HwaccelOutputFormat(input_index, method));
        }
        if let Some(format) = self.direct_format(encoding) {
            output_args.add(Arg::PixelFormat(output_index, format));
            return FramePlan {
                input_args,
                output_args,
            };
        }

        let mut filters = Vec::new();
        let memory_format = if method == "none" {
            source.name.as_str()
        } else {
            let download = self.download_format(source);
            filters.push(format!("hwdownload,format={download}"));
            download
        };

        if memory_format != target {
            if method == "none" {
                filters.push(format!("format={memory_format}"));
            }
            filters.push(format!("scale=iw:ih,format={target}"));
        }

        let output_format = if matches!(encoding, VideoEncoding::Vaapi { .. }) {
            filters.push("hwupload".to_owned());
            "vaapi"
        } else {
            target
        };

        if !filters.is_empty() {
            output_args.add(Arg::Filter(output_index, &filters.join(",")));
        }

        output_args.add(Arg::PixelFormat(output_index, output_format));
        FramePlan {
            input_args,
            output_args,
        }
    }

    pub(super) fn direct_format(&self, encoding: &VideoEncoding) -> Option<&'static str> {
        match (self, encoding) {
            (DecodingBackend::Vaapi(device), VideoEncoding::Vaapi { device: target, .. })
                if device.as_deref().is_none_or(|d| d == target.as_os_str()) =>
            {
                Some("vaapi")
            }
            (DecodingBackend::Cuda(_), VideoEncoding::Nvenc { .. }) => Some("cuda"),
            (DecodingBackend::Qsv(_), VideoEncoding::Qsv { .. }) => Some("qsv"),
            _ => None,
        }
    }

    fn download_format<'a>(&self, source: &'a PixelFormat) -> &'a str {
        match self {
            Self::Software => unreachable!("software frames do not need downloading"),
            Self::Vaapi(_) | Self::Qsv(_) => pixel_format::vaapi_qsv_format(source),
            Self::Cuda(_) => {
                let Some(VideoSamples {
                    component_depths,
                    chroma:
                        Chroma::Yuv {
                            log2_width,
                            log2_height,
                        },
                    alpha: false,
                }) = source.samples()
                else {
                    return &source.name;
                };
                match (log2_width, log2_height, component_depths.as_slice()) {
                    (1, 1, [8, 8, 8]) => "nv12",
                    (1, 1, [10, 10, 10]) => "p010le",
                    (1, 1, [12, 12, 12]) => "p012le",
                    (1, 0, [8, 8, 8]) => "nv16",
                    (1, 0, [10, 10, 10]) => "p210le",
                    (1, 0, [12, 12, 12]) => "p212le",
                    (0, 0, [8, 8, 8]) => "yuv444p",
                    (0, 0, [10, 10, 10]) => "yuv444p10msble",
                    (0, 0, [12, 12, 12]) => "yuv444p12msble",
                    _ => &source.name,
                }
            }
        }
    }
}
