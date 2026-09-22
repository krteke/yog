use yog_core::ffmpeg::{decoding::DecodingBackend, encoding::VideoEncoding};

const X26X_PRESETS: &[&str] = &[
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
];
const QSV_PRESETS: &[&str] = &[
    "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow",
];
const NVENC_PRESETS: &[&str] = &["p1", "p2", "p3", "p4", "p5", "p6", "p7"];

#[derive(Debug, Clone, Copy)]
pub enum DecoderChoice {
    Software,
    Vaapi,
    Cuda,
    Qsv,
}

impl DecoderChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Software => "Software",
            Self::Vaapi => "VAAPI",
            Self::Cuda => "CUDA",
            Self::Qsv => "QSV",
        }
    }

    pub fn backend(self) -> DecodingBackend {
        match self {
            Self::Software => DecodingBackend::Software,
            Self::Vaapi => DecodingBackend::Vaapi(None),
            Self::Cuda => DecodingBackend::Cuda(None),
            Self::Qsv => DecodingBackend::Qsv(None),
        }
    }
}

pub const DECODERS: &[DecoderChoice] = &[
    DecoderChoice::Software,
    DecoderChoice::Vaapi,
    DecoderChoice::Cuda,
    DecoderChoice::Qsv,
];

#[derive(Debug, Clone, Copy)]
enum Presets {
    Named(&'static [&'static str]),
    Numbered { first: u8, last: u8 },
    None,
}

impl Presets {
    fn len(self) -> usize {
        match self {
            Self::Named(values) => values.len(),
            Self::Numbered { first, last } => usize::from(last - first) + 1,
            Self::None => 0,
        }
    }

    fn label(self, index: usize) -> String {
        match self {
            Self::Named(values) => values[index].to_owned(),
            Self::Numbered { first, .. } => (first + index as u8).to_string(),
            Self::None => "—".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EncoderChoice {
    name: &'static str,
    presets: Presets,
}

impl EncoderChoice {
    pub fn encoding(self) -> VideoEncoding {
        self.name
            .parse()
            .expect("TUI encoder names must be accepted by yog-core")
    }

    pub fn configured(self, preset: usize) -> VideoEncoding {
        let mut encoding = self.encoding();
        if self.preset_count() > 0 {
            encoding
                .try_set_preset(&self.preset_label(preset))
                .expect("TUI preset choices must be accepted by yog-core");
        }
        encoding
    }

    pub fn label(self) -> String {
        self.encoding().to_string()
    }

    pub fn preset_count(self) -> usize {
        self.presets.len()
    }

    pub fn preset_label(self, index: usize) -> String {
        self.presets.label(index)
    }

    pub fn is_nvenc(self) -> bool {
        matches!(self.encoding(), VideoEncoding::Nvenc { .. })
    }
}

pub const ENCODERS: &[EncoderChoice] = &[
    EncoderChoice {
        name: "libx264",
        presets: Presets::Named(X26X_PRESETS),
    },
    EncoderChoice {
        name: "libx265",
        presets: Presets::Named(X26X_PRESETS),
    },
    EncoderChoice {
        name: "libsvtav1",
        presets: Presets::Numbered { first: 0, last: 13 },
    },
    EncoderChoice {
        name: "libaom-av1",
        presets: Presets::Numbered { first: 0, last: 8 },
    },
    EncoderChoice {
        name: "librav1e",
        presets: Presets::Numbered { first: 0, last: 10 },
    },
    EncoderChoice {
        name: "h264_nvenc",
        presets: Presets::Named(NVENC_PRESETS),
    },
    EncoderChoice {
        name: "hevc_nvenc",
        presets: Presets::Named(NVENC_PRESETS),
    },
    EncoderChoice {
        name: "av1_nvenc",
        presets: Presets::Named(NVENC_PRESETS),
    },
    EncoderChoice {
        name: "h264_qsv",
        presets: Presets::Named(QSV_PRESETS),
    },
    EncoderChoice {
        name: "hevc_qsv",
        presets: Presets::Named(QSV_PRESETS),
    },
    EncoderChoice {
        name: "av1_qsv",
        presets: Presets::Named(QSV_PRESETS),
    },
    EncoderChoice {
        name: "h264_vaapi",
        presets: Presets::None,
    },
    EncoderChoice {
        name: "hevc_vaapi",
        presets: Presets::None,
    },
    EncoderChoice {
        name: "av1_vaapi",
        presets: Presets::None,
    },
];

pub fn cycle(current: usize, len: usize, direction: isize) -> usize {
    debug_assert!(len > 0);
    if direction < 0 {
        current.checked_sub(1).unwrap_or(len - 1)
    } else {
        (current + 1) % len
    }
}
