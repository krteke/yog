use std::{ffi::OsString, num::NonZeroU64, path::PathBuf};

use yog_core::ffmpeg::{
    decoding::DecodingBackend,
    encoding::{DEFAULT_VAAPI_DEVICE, NvencMultipass, RateControl, VideoEncoding},
};
use yog_runtime::parse_quality_points;

use crate::text_input::TextInput;

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
enum DecoderChoice {
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
}

const DECODERS: &[DecoderChoice] = &[
    DecoderChoice::Software,
    DecoderChoice::Vaapi,
    DecoderChoice::Cuda,
    DecoderChoice::Qsv,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderDraft {
    choice: usize,
    device: TextInput,
}

impl DecoderDraft {
    pub fn new(choice: usize) -> Self {
        Self {
            choice,
            device: TextInput::default(),
        }
    }

    pub fn label(&self) -> &'static str {
        DECODERS[self.choice].label()
    }

    pub fn has_device(&self) -> bool {
        !matches!(DECODERS[self.choice], DecoderChoice::Software)
    }

    pub fn device(&self, editing: bool) -> String {
        self.device.display(editing)
    }

    pub fn device_input(&mut self) -> &mut TextInput {
        &mut self.device
    }

    pub fn adjust(&mut self, direction: isize) {
        self.choice = cycle(self.choice, DECODERS.len(), direction);
    }

    pub fn backend(&self) -> DecodingBackend {
        let device = (!self.device.value().is_empty()).then(|| OsString::from(self.device.value()));
        match DECODERS[self.choice] {
            DecoderChoice::Software => DecodingBackend::Software,
            DecoderChoice::Vaapi => DecodingBackend::Vaapi(device),
            DecoderChoice::Cuda => DecodingBackend::Cuda(device),
            DecoderChoice::Qsv => DecodingBackend::Qsv(device),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateMode {
    Default,
    Quality,
    Bitrate,
}

impl RateMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Quality => "Quality",
            Self::Bitrate => "Bitrate",
        }
    }
}

const MULTIPASS: &[(&str, Option<NvencMultipass>)] = &[
    ("Default", None),
    ("Disabled", Some(NvencMultipass::Disabled)),
    (
        "Quarter resolution",
        Some(NvencMultipass::QuarterResolution),
    ),
    ("Full resolution", Some(NvencMultipass::FullResolution)),
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
struct EncoderChoice {
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

const ENCODERS: &[EncoderChoice] = &[
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingDraft {
    encoder: usize,
    preset: usize,
    multipass: usize,
    rate: RateMode,
    quality: u8,
    bitrate: TextInput,
    quality_points: TextInput,
    device: TextInput,
}

impl EncodingDraft {
    pub fn new(
        encoder: usize,
        preset: usize,
        quality: u8,
        quality_points: impl Into<String>,
    ) -> Self {
        let choice = ENCODERS[encoder];
        let range = choice.encoding().quality_range();
        Self {
            encoder,
            preset: preset.min(choice.preset_count().saturating_sub(1)),
            multipass: 0,
            rate: RateMode::Quality,
            quality: quality.clamp(*range.start(), *range.end()),
            bitrate: TextInput::default(),
            quality_points: TextInput::new(quality_points),
            device: TextInput::new(DEFAULT_VAAPI_DEVICE),
        }
    }

    fn encoder(&self) -> EncoderChoice {
        ENCODERS[self.encoder]
    }

    pub fn encoder_label(&self) -> String {
        self.encoder().label()
    }

    pub fn preset_label(&self) -> String {
        self.encoder().preset_label(self.preset)
    }

    pub fn preset_count(&self) -> usize {
        self.encoder().preset_count()
    }

    pub fn is_nvenc(&self) -> bool {
        self.encoder().is_nvenc()
    }

    pub fn is_vaapi(&self) -> bool {
        matches!(self.encoder().encoding(), VideoEncoding::Vaapi { .. })
    }

    pub fn rate(&self) -> RateMode {
        self.rate
    }

    pub fn rate_label(&self) -> &'static str {
        self.rate.label()
    }

    pub fn quality_label(&self) -> String {
        format!(
            "{} {}",
            self.encoder().encoding().quality_parameter(),
            self.quality
        )
    }

    pub fn bitrate(&self, editing: bool) -> String {
        let value = self.bitrate.display(editing);
        if editing || self.bitrate.value().is_empty() {
            value
        } else {
            format!("{value} bps")
        }
    }

    pub fn bitrate_input(&mut self) -> &mut TextInput {
        &mut self.bitrate
    }

    pub fn multipass_label(&self) -> &'static str {
        MULTIPASS[self.multipass].0
    }

    pub fn quality_points_label(&self, editing: bool) -> String {
        let parameter = self.encoder().encoding().quality_parameter();
        if self.quality_points.value().is_empty() && !editing {
            format!("{parameter} all")
        } else {
            format!("{parameter} {}", self.quality_points.display(editing))
        }
    }

    pub fn quality_points_input(&mut self) -> &mut TextInput {
        &mut self.quality_points
    }

    pub fn qualities(&self) -> Result<Vec<u8>, String> {
        parse_quality_points(self.quality_points.value())
    }

    pub fn device(&self, editing: bool) -> String {
        self.device.display(editing)
    }

    pub fn device_input(&mut self) -> &mut TextInput {
        &mut self.device
    }

    pub fn adjust_encoder(&mut self, direction: isize) {
        self.encoder = cycle(self.encoder, ENCODERS.len(), direction);
        let encoder = self.encoder();
        self.preset = self.preset.min(encoder.preset_count().saturating_sub(1));
        self.multipass = 0;
        let range = encoder.encoding().quality_range();
        self.quality = self.quality.clamp(*range.start(), *range.end());
        self.quality_points = TextInput::new(format!("{}-{}", range.start(), range.end()));
    }

    pub fn adjust_preset(&mut self, direction: isize) {
        self.preset = cycle(self.preset, self.preset_count(), direction);
    }

    pub fn adjust_rate(&mut self, direction: isize) {
        let current = match self.rate {
            RateMode::Default => 0,
            RateMode::Quality => 1,
            RateMode::Bitrate => 2,
        };
        self.rate = match cycle(current, 3, direction) {
            0 => RateMode::Default,
            1 => RateMode::Quality,
            _ => RateMode::Bitrate,
        };
    }

    pub fn adjust_quality(&mut self, direction: isize) {
        let range = self.encoder().encoding().quality_range();
        self.quality = if direction < 0 {
            self.quality.saturating_sub(1).max(*range.start())
        } else {
            self.quality.saturating_add(1).min(*range.end())
        };
    }

    pub fn adjust_multipass(&mut self, direction: isize) {
        self.multipass = cycle(self.multipass, MULTIPASS.len(), direction);
    }

    pub fn build(&self) -> VideoEncoding {
        let mut encoding = self.encoder().configured(self.preset);
        match &mut encoding {
            VideoEncoding::Nvenc { multipass, .. } => {
                *multipass = MULTIPASS[self.multipass].1;
            }
            VideoEncoding::Vaapi { device, .. } => {
                *device = PathBuf::from(self.device.value());
            }
            _ => {}
        }
        encoding
    }

    pub fn build_with_rate(&self) -> Result<VideoEncoding, String> {
        let mut encoding = self.build();
        let rate = match self.rate {
            RateMode::Default => None,
            RateMode::Quality => Some(RateControl::Quality(self.quality)),
            RateMode::Bitrate => Some(RateControl::Bitrate(
                self.bitrate
                    .value()
                    .parse::<NonZeroU64>()
                    .map_err(|_| "Bitrate must be an integer greater than zero".to_owned())?,
            )),
        };
        encoding.set_rate(rate);
        Ok(encoding)
    }
}

pub fn cycle(current: usize, len: usize, direction: isize) -> usize {
    debug_assert!(len > 0);
    if direction < 0 {
        current.checked_sub(1).unwrap_or(len - 1)
    } else {
        (current + 1) % len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_draft_builds_bitrate_and_nvenc_multipass() {
        let mut draft = EncodingDraft::new(5, 3, 23, "10-30");
        draft.rate = RateMode::Bitrate;
        draft.bitrate = TextInput::new("4000000");
        draft.adjust_multipass(-1);

        let encoding = draft.build_with_rate().unwrap();
        assert!(matches!(
            encoding.rate(),
            Some(RateControl::Bitrate(value)) if value.get() == 4_000_000
        ));
        assert_eq!(encoding.preset().as_deref(), Some("p4"));
        assert_eq!(encoding.multipass(), Some("fullres"));
    }

    #[test]
    fn hardware_devices_reach_the_runtime_types() {
        let mut decoding = DecoderDraft::new(2);
        decoding.device = TextInput::new("1");
        let mut encoding = EncodingDraft::new(11, 0, 23, "10-30");
        encoding.device = TextInput::new("/dev/dri/renderD129");

        assert!(matches!(
            decoding.backend(),
            DecodingBackend::Cuda(Some(device)) if device == "1"
        ));
        assert!(matches!(
            encoding.build(),
            VideoEncoding::Vaapi { device, .. }
                if device == std::path::Path::new("/dev/dri/renderD129")
        ));
    }

    #[test]
    fn encoding_draft_expands_discrete_quality_points() {
        let encoding = EncodingDraft::new(1, 5, 23, "30,10-12,30");

        assert_eq!(encoding.qualities().unwrap(), vec![10, 11, 12, 30]);
    }
}
