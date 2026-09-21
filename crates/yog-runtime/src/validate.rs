use crate::{
    Command, Operation,
    emulate::{Candidate, EmulationOptions},
};
use anyhow::Context;
use std::path::{Path, PathBuf};
use yog_core::ffmpeg::{
    decoding::DecodingBackend,
    encoding::{RateControl, VideoEncoding},
    plan::VideoAction,
};

/// Optional validation for frontends that cannot enforce these invariants while
/// constructing their values.
///
/// Implemented for [`Command`], [`Candidate`], [`VideoEncoding`] and
/// [`DecodingBackend`], so callers can validate only the layer they need.
pub trait Validate {
    fn validate(&self) -> anyhow::Result<()>;
}

impl Validate for Command {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.request.input.as_os_str().is_empty(),
            "an input path is required"
        );
        self.request.decoding.validate()?;

        match &self.operation {
            Operation::Transcode => {
                if let VideoAction::Encode(encoding) = &self.request.video {
                    encoding.validate()?;
                }
                anyhow::ensure!(
                    !self.request.output.as_os_str().is_empty(),
                    "transcoding requires an output path"
                );
                anyhow::ensure!(
                    !same_path(&self.request.input, &self.request.output),
                    "input and output paths must be different"
                );
            }
            Operation::Predict => {
                anyhow::ensure!(
                    self.request.output.as_os_str().is_empty(),
                    "prediction does not accept a video output"
                );
                let VideoAction::Encode(encoding) = &self.request.video else {
                    anyhow::bail!("prediction requires a video encoder");
                };
                encoding.validate()?;
            }
            Operation::Emulate(options) => {
                validate_emulation(self, options)?;
            }
        }

        Ok(())
    }
}

fn validate_emulation(command: &Command, options: &EmulationOptions) -> anyhow::Result<()> {
    anyhow::ensure!(
        !command.recursive,
        "emulation does not support recursive processing"
    );
    anyhow::ensure!(
        command.request.output.as_os_str().is_empty(),
        "emulation does not accept a video output"
    );
    anyhow::ensure!(
        options.png.is_some() || options.svg.is_some(),
        "emulation requires a PNG or SVG output"
    );
    if let (Some(png), Some(svg)) = (&options.png, &options.svg) {
        anyhow::ensure!(
            !same_path(png, svg),
            "PNG and SVG outputs must use different paths"
        );
    }
    if let Some(png) = &options.png {
        anyhow::ensure!(
            !same_path(&command.request.input, png),
            "PNG output must differ from the input path"
        );
    }
    if let Some(svg) = &options.svg {
        anyhow::ensure!(
            !same_path(&command.request.input, svg),
            "SVG output must differ from the input path"
        );
    }

    let encoding = match &command.request.video {
        VideoAction::Encode(encoding) => Some(encoding),
        VideoAction::Copy => None,
    };
    anyhow::ensure!(
        encoding.is_some() || !options.candidates.is_empty(),
        "emulation requires an encoder or at least one candidate"
    );
    anyhow::ensure!(
        encoding.is_none() || options.candidates.is_empty(),
        "emulation does not accept both an encoder and candidates"
    );

    if let Some(encoding) = encoding {
        encoding.validate()?;
        anyhow::ensure!(
            encoding.rate().is_none(),
            "emulation does not accept a fixed quality or bitrate"
        );
        validate_qualities(encoding, &options.qualities)?;
        return Ok(());
    }

    anyhow::ensure!(
        options.qualities.is_empty(),
        "emulation candidates define their own quality ranges"
    );
    anyhow::ensure!(
        matches!(command.request.decoding, DecodingBackend::Software),
        "emulation candidates specify their own decoder"
    );
    for (index, candidate) in options.candidates.iter().enumerate() {
        candidate
            .validate()
            .with_context(|| format!("invalid emulation candidate #{}", index + 1))?;
    }

    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (comparable_path(left), comparable_path(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn comparable_path(path: &Path) -> Option<PathBuf> {
    path.canonicalize()
        .or_else(|_| std::path::absolute(path))
        .ok()
}

fn validate_qualities(encoding: &VideoEncoding, qualities: &[u8]) -> anyhow::Result<()> {
    let supported = encoding.quality_range();
    if let Some(quality) = qualities
        .iter()
        .copied()
        .find(|quality| !supported.contains(quality))
    {
        anyhow::bail!(
            "{} quality must be within {}..={}, got {quality}",
            encoding.name(),
            supported.start(),
            supported.end(),
        );
    }
    Ok(())
}

impl Validate for Candidate {
    fn validate(&self) -> anyhow::Result<()> {
        self.decoding.validate()?;
        self.encoding.validate()?;
        anyhow::ensure!(
            self.encoding.rate().is_none(),
            "emulation candidates do not accept a fixed quality or bitrate"
        );
        validate_qualities(&self.encoding, &self.qualities)
    }
}

impl Validate for VideoEncoding {
    fn validate(&self) -> anyhow::Result<()> {
        if let Some(RateControl::Quality(quality)) = self.rate() {
            let range = self.quality_range();

            anyhow::ensure!(
                range.contains(&quality),
                "{} quality must be in {}..={}, got {quality}",
                self.name(),
                range.start(),
                range.end(),
            );
        }

        match self {
            Self::SvtAv1 {
                preset: Some(preset),
                ..
            } => anyhow::ensure!(*preset <= 13, "SVT-AV1 preset must be within 0..=13"),
            Self::AomAv1 {
                cpu_used: Some(cpu_used),
                ..
            } => anyhow::ensure!(*cpu_used <= 8, "AOM-AV1 cpu-used must be within 0..=8"),
            Self::Rav1e {
                speed: Some(speed), ..
            } => anyhow::ensure!(*speed <= 10, "rav1e speed must be within 0..=10"),
            Self::Vaapi { device, .. } => anyhow::ensure!(
                !device.as_os_str().is_empty(),
                "VAAPI encoding device must not be empty"
            ),
            _ => {}
        }

        Ok(())
    }
}

impl Validate for DecodingBackend {
    fn validate(&self) -> anyhow::Result<()> {
        let device = match self {
            Self::Software | Self::Vaapi(None) | Self::Cuda(None) | Self::Qsv(None) => {
                return Ok(());
            }
            Self::Vaapi(Some(device)) | Self::Cuda(Some(device)) | Self::Qsv(Some(device)) => {
                device
            }
        };
        anyhow::ensure!(!device.is_empty(), "decoding device must not be empty");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yog_core::ffmpeg::{
        encoding::{Preset, RateControl},
        plan::TranscodeRequest,
    };

    fn x264(rate: Option<RateControl>) -> VideoEncoding {
        VideoEncoding::X264 {
            rate,
            preset: Some(Preset::Medium),
        }
    }

    fn candidate(qualities: Vec<u8>) -> Candidate {
        Candidate {
            decoding: DecodingBackend::Software,
            encoding: x264(None),
            qualities,
        }
    }

    fn emulate(
        request: TranscodeRequest,
        qualities: Vec<u8>,
        candidates: Vec<Candidate>,
    ) -> Command {
        Command {
            request,
            operation: Operation::Emulate(EmulationOptions {
                png: Some("plot.png".into()),
                svg: None,
                qualities,
                candidates,
            }),
            recursive: false,
        }
    }

    #[track_caller]
    fn assert_error(command: Command, expected: &str) {
        assert_eq!(format!("{:#}", command.validate().unwrap_err()), expected);
    }

    #[test]
    fn command_validation_requires_an_input_path() {
        assert_error(
            Command {
                request: TranscodeRequest::new("", "output.mkv"),
                operation: Operation::Transcode,
                recursive: false,
            },
            "an input path is required",
        );
    }

    #[test]
    fn command_validation_requires_a_transcode_output_path() {
        assert_error(
            Command {
                request: TranscodeRequest::new("input.mkv", ""),
                operation: Operation::Transcode,
                recursive: false,
            },
            "transcoding requires an output path",
        );
    }

    #[test]
    fn command_validation_rejects_overwriting_the_input_with_video_output() {
        assert_error(
            Command {
                request: TranscodeRequest::new("input.mkv", "input.mkv"),
                operation: Operation::Transcode,
                recursive: false,
            },
            "input and output paths must be different",
        );
    }

    #[test]
    fn command_validation_rejects_empty_hardware_device_names() {
        assert_error(
            Command {
                request: TranscodeRequest::new("input.mkv", "output.mkv")
                    .with_decoding(DecodingBackend::Cuda(Some("".into()))),
                operation: Operation::Transcode,
                recursive: false,
            },
            "decoding device must not be empty",
        );

        assert_error(
            Command {
                request: TranscodeRequest::new("input.mkv", "output.mkv").with_video(
                    VideoAction::Encode(VideoEncoding::Vaapi {
                        codec: yog_core::ffmpeg::encoding::VideoCodec::Av1,
                        rate: None,
                        device: "".into(),
                    }),
                ),
                operation: Operation::Transcode,
                recursive: false,
            },
            "VAAPI encoding device must not be empty",
        );
    }

    #[test]
    fn command_validation_rejects_prediction_output_and_copy_mode() {
        assert_error(
            Command {
                request: TranscodeRequest::new("input.mkv", "output.mkv")
                    .with_video(VideoAction::Encode(x264(None))),
                operation: Operation::Predict,
                recursive: false,
            },
            "prediction does not accept a video output",
        );
        assert_error(
            Command {
                request: TranscodeRequest::new("input.mkv", ""),
                operation: Operation::Predict,
                recursive: false,
            },
            "prediction requires a video encoder",
        );
    }

    #[test]
    fn command_validation_rejects_emulation_output_collisions() {
        let request =
            TranscodeRequest::new("input.mkv", "").with_video(VideoAction::Encode(x264(None)));
        let mut command = emulate(request, vec![20], Vec::new());
        let Operation::Emulate(options) = &mut command.operation else {
            unreachable!()
        };
        options.svg = Some("plot.png".into());
        assert_error(command, "PNG and SVG outputs must use different paths");

        let request =
            TranscodeRequest::new("input.mkv", "").with_video(VideoAction::Encode(x264(None)));
        let mut command = emulate(request, vec![20], Vec::new());
        let Operation::Emulate(options) = &mut command.operation else {
            unreachable!()
        };
        options.png = Some("input.mkv".into());
        assert_error(command, "PNG output must differ from the input path");
    }

    #[test]
    fn command_validation_requires_an_emulation_chart_output() {
        let request =
            TranscodeRequest::new("input.mkv", "").with_video(VideoAction::Encode(x264(None)));
        let mut command = emulate(request, vec![20], Vec::new());
        let Operation::Emulate(options) = &mut command.operation else {
            unreachable!()
        };
        options.png = None;
        assert_error(command, "emulation requires a PNG or SVG output");
    }

    #[test]
    fn command_validation_rejects_recursive_emulation() {
        let request =
            TranscodeRequest::new("input.mkv", "").with_video(VideoAction::Encode(x264(None)));
        let mut command = emulate(request, vec![20], Vec::new());
        command.recursive = true;
        assert_error(command, "emulation does not support recursive processing");
    }

    #[test]
    fn single_encoder_emulation_accepts_supported_quality_points() {
        let request =
            TranscodeRequest::new("input.mkv", "").with_video(VideoAction::Encode(x264(None)));
        emulate(request, vec![18, 20, 22], Vec::new())
            .validate()
            .unwrap();
    }

    #[test]
    fn candidate_emulation_accepts_independent_decoders_and_quality_points() {
        let mut hardware = candidate(vec![20, 22]);
        hardware.decoding = DecodingBackend::Cuda(Some("0".into()));
        emulate(
            TranscodeRequest::new("input.mkv", ""),
            Vec::new(),
            vec![hardware],
        )
        .validate()
        .unwrap();
    }

    #[test]
    fn emulation_rejects_an_encoder_together_with_candidates() {
        let request =
            TranscodeRequest::new("input.mkv", "").with_video(VideoAction::Encode(x264(None)));
        assert_error(
            emulate(request, Vec::new(), vec![candidate(vec![20])]),
            "emulation does not accept both an encoder and candidates",
        );
    }

    #[test]
    fn candidate_emulation_rejects_a_shared_quality_range() {
        assert_error(
            emulate(
                TranscodeRequest::new("input.mkv", ""),
                vec![18, 20],
                vec![candidate(vec![22])],
            ),
            "emulation candidates define their own quality ranges",
        );
    }

    #[test]
    fn candidate_emulation_rejects_a_shared_decoder() {
        let request =
            TranscodeRequest::new("input.mkv", "").with_decoding(DecodingBackend::Vaapi(None));
        assert_error(
            emulate(request, Vec::new(), vec![candidate(vec![20])]),
            "emulation candidates specify their own decoder",
        );
    }

    #[test]
    fn emulation_rejects_missing_encoder_and_candidates() {
        assert_error(
            emulate(
                TranscodeRequest::new("input.mkv", ""),
                Vec::new(),
                Vec::new(),
            ),
            "emulation requires an encoder or at least one candidate",
        );
    }

    #[test]
    fn single_encoder_emulation_rejects_an_unsupported_quality() {
        let request =
            TranscodeRequest::new("input.mkv", "").with_video(VideoAction::Encode(x264(None)));
        assert_error(
            emulate(request, vec![52], Vec::new()),
            "libx264 quality must be within 0..=51, got 52",
        );
    }

    #[test]
    fn candidate_emulation_identifies_the_invalid_candidate() {
        assert_error(
            emulate(
                TranscodeRequest::new("input.mkv", ""),
                Vec::new(),
                vec![candidate(vec![20]), candidate(vec![52])],
            ),
            "invalid emulation candidate #2: libx264 quality must be within 0..=51, got 52",
        );
    }

    #[test]
    fn candidate_emulation_rejects_a_fixed_rate() {
        let mut fixed = candidate(vec![20]);
        fixed.encoding = x264(Some(RateControl::Quality(20)));
        assert_error(
            emulate(
                TranscodeRequest::new("input.mkv", ""),
                Vec::new(),
                vec![fixed],
            ),
            "invalid emulation candidate #1: emulation candidates do not accept a fixed quality or bitrate",
        );
    }

    #[test]
    fn encoding_validation_checks_numeric_presets() {
        let encoding = VideoEncoding::SvtAv1 {
            rate: None,
            preset: Some(14),
        };
        assert_eq!(
            encoding.validate().unwrap_err().to_string(),
            "SVT-AV1 preset must be within 0..=13"
        );
    }
}
