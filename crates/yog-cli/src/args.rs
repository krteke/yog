use clap::{ArgGroup, CommandFactory, Parser};
use std::{num::NonZeroU32, ops::RangeInclusive, path::PathBuf, str::FromStr};
use yog_core::ffmpeg::{
    plan::{Container, TranscodeRequest, VideoAction},
    vmaf::VmafOptions,
};

use crate::decoding::DecodingArgs;

#[derive(Debug, Parser)]
#[command(
    version,
    about,
    group(ArgGroup::new("emulation_output").args(["png", "svg"]).multiple(true))
)]
pub struct Args {
    pub input: PathBuf,
    #[arg(short, long, global = true)]
    pub recursive: bool,
    #[arg(long, global = true, conflicts_with = "verify")]
    pub predict: bool,
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[command(flatten)]
    pub emulation: EmulationArgs,
    #[arg(short, long, global = true)]
    pub output: Option<PathBuf>,
    #[arg(short = 'C', long, global = true)]
    pub container: Option<Container>,
    #[command(flatten)]
    pub decoding: DecodingArgs,
    #[arg(short = 'O', long, global = true)]
    pub overwrite: bool,
    #[command(flatten)]
    pub execution: ExecutionOptions,
    #[command(subcommand)]
    pub video: Option<VideoAction>,
}

#[derive(Debug, clap::Args)]
pub struct EmulationArgs {
    #[arg(
        long,
        global = true,
        conflicts_with_all = ["predict", "recursive", "verify", "vmaf"],
        requires = "emulation_output"
    )]
    pub emulate: bool,
    #[arg(long, global = true, requires = "emulate", group = "emulation_output")]
    pub png: Option<PathBuf>,
    #[arg(long, global = true, requires = "emulate", group = "emulation_output")]
    pub svg: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        value_name = "MIN,MAX",
        value_parser = parse_quality_range,
        requires = "emulate"
    )]
    pub range: Option<RangeInclusive<u8>>,
}

#[derive(Debug, clap::Args)]
pub struct ExecutionOptions {
    #[arg(long, default_value = "ffmpeg", global = true)]
    pub ffmpeg: PathBuf,
    #[arg(long, default_value = "ffprobe", global = true)]
    pub ffprobe: PathBuf,
    #[arg(long, short, global = true)]
    pub timeout: Option<u64>,
    #[arg(long, global = true)]
    pub verbose: bool,
    #[arg(short, long, global = true)]
    pub verify: bool,
    #[arg(
        long,
        global = true,
        value_name = "full|N_SUBSAMPLE",
        num_args = 0..=1,
        default_missing_value = "full",
        require_equals = true,
        conflicts_with = "predict"
    )]
    pub vmaf: Option<VmafMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmafMode {
    Full,
    Subsample(NonZeroU32),
}

impl FromStr for VmafMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("full") {
            return Ok(Self::Full);
        }
        value
            .parse::<NonZeroU32>()
            .map(Self::Subsample)
            .map_err(|_| "expected 'full' or an integer greater than 0".to_owned())
    }
}

impl From<VmafMode> for VmafOptions {
    fn from(value: VmafMode) -> Self {
        Self {
            n_subsample: match value {
                VmafMode::Full => None,
                VmafMode::Subsample(value) => Some(value),
            },
        }
    }
}

fn parse_quality_range(value: &str) -> Result<RangeInclusive<u8>, String> {
    let Some((start, end)) = value.split_once(',') else {
        return Err("expected MIN,MAX".to_owned());
    };
    let start = start
        .trim()
        .parse::<u8>()
        .map_err(|_| "MIN must be an integer from 0 to 255".to_owned())?;
    let end = end
        .trim()
        .parse::<u8>()
        .map_err(|_| "MAX must be an integer from 0 to 255".to_owned())?;
    if start > end {
        return Err("MIN must not exceed MAX".to_owned());
    }

    Ok(start..=end)
}

impl Args {
    pub fn into_request(self) -> Result<(TranscodeRequest, ExecutionOptions), clap::Error> {
        let mut request = TranscodeRequest::new(self.input, self.output.unwrap_or_default())
            .with_video(self.video.unwrap_or_default())
            .with_decoding(
                self.decoding
                    .try_into()
                    .map_err(|error: clap::Error| error.format(&mut Self::command()))?,
            )
            .with_overwrite(self.overwrite);
        if let Some(container) = self.container {
            request = request.with_container(container);
        }

        Ok((request, self.execution))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::Validate;
    use clap::{CommandFactory, error::ErrorKind};
    use yog_core::ffmpeg::{
        decoding::DecodingBackend,
        encoding::{NvencMultipass, NvencPreset, Preset, RateControl, VideoCodec, VideoEncoding},
    };

    #[test]
    fn modes_enforce_option_scope_required_fields_and_rate_exclusion() {
        Args::command().debug_assert();
        for (flags, kind) in [
            (
                vec!["--encode-x264", "--quality", "23", "--bitrate", "1000000"],
                ErrorKind::ArgumentConflict,
            ),
            (
                vec!["--copy", "--quality", "23"],
                ErrorKind::UnknownArgument,
            ),
            (
                vec!["--encode-x264", "--multipass", "qres"],
                ErrorKind::UnknownArgument,
            ),
            (
                vec![
                    "--encode-vaapi",
                    "h264",
                    "--device",
                    "/dev/dri/renderD128",
                    "--preset",
                    "p4",
                ],
                ErrorKind::UnknownArgument,
            ),
            (vec!["--encode-nvenc"], ErrorKind::MissingRequiredArgument),
            (
                vec!["--encode-qsv", "h264", "--preset", "p4"],
                ErrorKind::InvalidValue,
            ),
            (
                vec!["--encode-aom-av1", "--cpu-used", "fast"],
                ErrorKind::ValueValidation,
            ),
            (
                vec!["--encode-rav1e", "--preset", "6"],
                ErrorKind::UnknownArgument,
            ),
            (
                vec!["--encode-x264", "--bitrate", "0"],
                ErrorKind::ValueValidation,
            ),
            (
                vec!["--encode-x264", "--quality", "256"],
                ErrorKind::ValueValidation,
            ),
            (
                vec!["--decode-cuda", "--decode-qsv", "--copy"],
                ErrorKind::ArgumentConflict,
            ),
            (
                vec!["--encode-nvenc", "hevc", "--encode-vaapi", "hevc"],
                ErrorKind::UnknownArgument,
            ),
            (
                vec!["--quality", "23", "--encode-x264"],
                ErrorKind::UnknownArgument,
            ),
            (vec!["--encode", "nvenc"], ErrorKind::UnknownArgument),
            (
                vec!["--predict", "--verify", "--encode-x264"],
                ErrorKind::ArgumentConflict,
            ),
        ] {
            let error = Args::try_parse_from(
                ["yog", "input", "-o", "output"]
                    .into_iter()
                    .chain(flags.iter().copied()),
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind, "{flags:?}: {error}");
        }
    }

    #[test]
    fn vmaf_selects_full_or_a_positive_subsample_interval() {
        for (flags, expected) in [
            (vec!["--vmaf", "--copy"], VmafMode::Full),
            (vec!["--vmaf=full", "--copy"], VmafMode::Full),
            (
                vec!["--vmaf=7", "--copy"],
                VmafMode::Subsample(NonZeroU32::new(7).unwrap()),
            ),
        ] {
            let (_, options) =
                Args::try_parse_from(["yog", "input", "-o", "output"].into_iter().chain(flags))
                    .unwrap()
                    .into_request()
                    .unwrap();
            assert_eq!(options.vmaf, Some(expected));
        }

        for flags in [
            vec!["--vmaf=0", "--copy"],
            vec!["--predict", "--vmaf", "--encode-x264"],
        ] {
            assert!(
                Args::try_parse_from(["yog", "input", "-o", "output"].into_iter().chain(flags),)
                    .is_err()
            );
        }

        let (_, options) =
            Args::try_parse_from(["yog", "--vmaf", "input", "-o", "output", "--copy"])
                .unwrap()
                .into_request()
                .unwrap();
        assert_eq!(options.vmaf, Some(VmafMode::Full));
    }

    #[test]
    fn emulation_requires_an_encoder_chart_output_and_supported_ordered_range() {
        let args = Args::try_parse_from([
            "yog",
            "input.mkv",
            "--emulate",
            "--png",
            "quality.png",
            "--svg",
            "quality.svg",
            "--range",
            "20,22",
            "--encode-x264",
        ])
        .unwrap()
        .validate()
        .unwrap();
        assert_eq!(args.emulation.range, Some(20..=22));
        assert!(args.output.is_none());

        for flags in [
            vec!["--emulate", "--encode-x264"],
            vec![
                "--emulate",
                "--png",
                "quality.png",
                "--range",
                "30,20",
                "--encode-x264",
            ],
        ] {
            assert!(
                Args::try_parse_from(
                    ["yog", "input.mkv"]
                        .into_iter()
                        .chain(flags.iter().copied()),
                )
                .is_err(),
                "{flags:?}"
            );
        }

        for (flags, expected) in [
            (
                vec!["--emulate", "--png", "quality.png", "--copy"],
                "--emulate requires a video encoder",
            ),
            (
                vec![
                    "--emulate",
                    "--png",
                    "quality.png",
                    "--range",
                    "50,52",
                    "--encode-x264",
                ],
                "libx264 quality range must be within 0..=51, got 50..=52",
            ),
            (
                vec![
                    "--emulate",
                    "--png",
                    "quality.png",
                    "--encode-x264",
                    "--quality",
                    "20",
                ],
                "--quality and --bitrate cannot be used with --emulate",
            ),
            (
                vec![
                    "--emulate",
                    "--png",
                    "quality.plot",
                    "--svg",
                    "quality.plot",
                    "--encode-x264",
                ],
                "--png and --svg must use different paths",
            ),
        ] {
            let error = Args::try_parse_from(
                ["yog", "input.mkv"]
                    .into_iter()
                    .chain(flags.iter().copied()),
            )
            .unwrap()
            .validate()
            .unwrap_err();
            assert_eq!(error.to_string(), expected, "{flags:?}");
        }
    }

    #[test]
    fn prediction_and_emulation_do_not_accept_a_video_output() {
        let prediction = Args::try_parse_from([
            "yog",
            "input.mkv",
            "--predict",
            "--encode-x264",
            "--quality",
            "23",
        ])
        .unwrap();
        assert!(prediction.output.is_none());

        for flags in [
            vec!["--predict", "--encode-x264"],
            vec!["--emulate", "--png", "quality.png", "--encode-x264"],
        ] {
            let error = Args::try_parse_from(
                ["yog", "input.mkv", "--output", "video.mkv"]
                    .into_iter()
                    .chain(flags),
            )
            .unwrap()
            .validate()
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("--output cannot be used with")
            );
        }
    }

    #[test]
    fn encoding_flags_can_be_reordered_inside_the_mode() {
        for encoding_flags in [
            vec!["--preset", "p4", "--multipass", "qres", "--quality", "27"],
            vec!["--quality", "27", "--multipass", "qres", "--preset", "p4"],
        ] {
            let (request, options) = Args::try_parse_from(
                [
                    "yog",
                    "input",
                    "-o",
                    "output",
                    "--decode-vaapi=/dev/dri/custom",
                    "-C",
                    "mp4",
                    "-O",
                    "--timeout",
                    "7",
                    "--encode-nvenc",
                    "hevc",
                ]
                .into_iter()
                .chain(encoding_flags),
            )
            .unwrap()
            .into_request()
            .unwrap();
            assert!(
                matches!(request.decoding, DecodingBackend::Vaapi(Some(device)) if device == "/dev/dri/custom")
            );
            assert!(matches!(
                request.video,
                VideoAction::Encode(VideoEncoding::Nvenc {
                    codec: VideoCodec::Hevc,
                    rate: Some(RateControl::Quality(27)),
                    preset: Some(NvencPreset::P4),
                    multipass: Some(NvencMultipass::QuarterResolution),
                })
            ));
            assert!(matches!(request.container, Some(Container::Mp4)));
            assert!(request.overwrite);
            assert_eq!(options.timeout, Some(7));
        }
    }

    #[test]
    fn global_options_work_before_after_and_across_the_mode() {
        let (request, _) = Args::try_parse_from([
            "yog",
            "input",
            "--encode-vaapi",
            "--decode-cuda",
            "av1",
            "-o",
            "output",
        ])
        .unwrap()
        .into_request()
        .unwrap();
        assert!(matches!(request.decoding, DecodingBackend::Cuda(None)));
        assert!(matches!(
            request.video,
            VideoAction::Encode(VideoEncoding::Vaapi {
                codec: VideoCodec::Av1,
                ..
            })
        ));
        for flags in [
            vec![
                "input",
                "-o",
                "output",
                "-C",
                "mp4",
                "-O",
                "--decode-cuda=0",
                "--timeout",
                "7",
                "--verbose",
                "--ffmpeg",
                "custom-ffmpeg",
                "--ffprobe",
                "custom-ffprobe",
                "--encode-nvenc",
                "hevc",
                "--quality",
                "27",
            ],
            vec![
                "input",
                "--encode-nvenc",
                "--quality",
                "27",
                "-o",
                "output",
                "-C",
                "mp4",
                "-O",
                "--decode-cuda=0",
                "--timeout",
                "7",
                "--verbose",
                "--ffmpeg",
                "custom-ffmpeg",
                "--ffprobe",
                "custom-ffprobe",
                "hevc",
            ],
            vec![
                "-C",
                "mp4",
                "input",
                "--decode-cuda=0",
                "--ffmpeg",
                "custom-ffmpeg",
                "--encode-nvenc",
                "--quality",
                "27",
                "-o",
                "output",
                "-O",
                "--timeout",
                "7",
                "hevc",
                "--verbose",
                "--ffprobe",
                "custom-ffprobe",
            ],
        ] {
            let (request, options) = Args::try_parse_from(["yog"].into_iter().chain(flags))
                .unwrap()
                .into_request()
                .unwrap();
            assert_eq!(request.output, PathBuf::from("output"));
            assert!(matches!(request.container, Some(Container::Mp4)));
            assert!(request.overwrite);
            assert!(
                matches!(request.decoding, DecodingBackend::Cuda(Some(device)) if device == "0")
            );
            assert!(matches!(
                request.video,
                VideoAction::Encode(VideoEncoding::Nvenc {
                    codec: VideoCodec::Hevc,
                    rate: Some(RateControl::Quality(27)),
                    ..
                })
            ));
            assert_eq!(options.timeout, Some(7));
            assert!(options.verbose);
            assert_eq!(options.ffmpeg, PathBuf::from("custom-ffmpeg"));
            assert_eq!(options.ffprobe, PathBuf::from("custom-ffprobe"));
        }
        for flags in [
            vec!["input"],
            vec!["input", "--copy"],
            vec!["input", "--encode-vaapi", "av1"],
        ] {
            let error = Args::try_parse_from(["yog"].into_iter().chain(flags))
                .unwrap()
                .validate()
                .unwrap_err();
            assert_eq!(error.to_string(), "--output is required");
        }
        for flags in [
            vec!["--decode-cuda", "--copy", "--decode-qsv"],
            vec!["--copy", "--decode-cuda", "--decode-qsv"],
        ] {
            let error =
                Args::try_parse_from(["yog", "input", "-o", "output"].into_iter().chain(flags))
                    .and_then(Args::into_request)
                    .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn omitted_mode_and_omitted_rate_keep_their_defaults() {
        for flags in [vec![], vec!["--copy"]] {
            let (request, _) =
                Args::try_parse_from(["yog", "input", "-o", "output"].into_iter().chain(flags))
                    .unwrap()
                    .into_request()
                    .unwrap();
            assert!(request.container.is_none());
            assert!(matches!(request.decoding, DecodingBackend::Software));
            assert!(matches!(request.video, VideoAction::Copy));
        }
        let (request, _) = Args::try_parse_from([
            "yog",
            "input",
            "-o",
            "output",
            "--encode-x264",
            "--preset",
            "medium",
        ])
        .unwrap()
        .into_request()
        .unwrap();
        assert!(matches!(
            request.video,
            VideoAction::Encode(VideoEncoding::X264 {
                rate: None,
                preset: Some(Preset::Medium)
            })
        ));
        let (request, _) = Args::try_parse_from([
            "yog",
            "input",
            "-o",
            "output",
            "--encode-x264",
            "--quality",
            "0",
        ])
        .unwrap()
        .into_request()
        .unwrap();
        assert!(matches!(
            request.video,
            VideoAction::Encode(VideoEncoding::X264 {
                rate: Some(RateControl::Quality(0)),
                ..
            })
        ));
        let (request, _) =
            Args::try_parse_from(["yog", "input", "-o", "output", "--decode-cuda", "--copy"])
                .unwrap()
                .into_request()
                .unwrap();
        assert!(matches!(request.video, VideoAction::Copy));
        assert!(matches!(request.decoding, DecodingBackend::Cuda(None)));
    }

    #[test]
    fn every_output_container_has_a_cli_value() {
        for (value, expected) in [
            ("mkv", Container::Matroska),
            ("mp4", Container::Mp4),
            ("mov", Container::Mov),
            ("m4a", Container::M4a),
            ("3gp", Container::ThreeGp),
            ("3g2", Container::ThreeG2),
            ("f4v", Container::F4v),
            ("ismv", Container::Ismv),
            ("psp", Container::Psp),
            ("webm", Container::Webm),
            ("ts", Container::MpegTs),
            ("m2ts", Container::M2ts),
            ("avi", Container::Avi),
            ("flv", Container::Flv),
            ("asf", Container::Asf),
            ("wmv", Container::Wmv),
            ("mpg", Container::MpegPs),
            ("mpeg", Container::MpegPs),
            ("vob", Container::Vob),
            ("ogg", Container::Ogg),
            ("ogv", Container::Ogv),
        ] {
            let (request, _) =
                Args::try_parse_from(["yog", "input", "-o", "output", "-C", value, "--copy"])
                    .unwrap()
                    .into_request()
                    .unwrap();
            assert_eq!(request.container, Some(expected), "{value}");
        }
    }

    #[test]
    fn help_displays_only_the_selected_modes_flags_without_requiring_files() {
        let help = Args::try_parse_from(["yog", "--encode-nvenc", "--help"]).unwrap_err();
        assert_eq!(help.kind(), ErrorKind::DisplayHelp);
        let text = help.to_string();
        assert!(text.contains("--multipass"));
        assert!(text.contains("p7"));
        assert!(text.contains("--quality"));
        assert!(!text.contains("--device"));
        assert!(!text.contains("Commands:"));
        let help = Args::try_parse_from(["yog", "--encode-vaapi", "--help"]).unwrap_err();
        assert_eq!(help.kind(), ErrorKind::DisplayHelp);
        assert!(help.to_string().contains("--device"));
        assert!(!help.to_string().contains("--preset"));
        for flags in [
            vec!["--encode-vaapi", "--help"],
            vec![
                "input",
                "--encode-vaapi",
                "av1",
                "-o",
                "output.mkv",
                "--help",
            ],
        ] {
            let help = Args::try_parse_from(["yog"].into_iter().chain(flags)).unwrap_err();
            assert_eq!(help.kind(), ErrorKind::DisplayHelp);
            let text = help.to_string();
            let usage = text
                .lines()
                .find(|line| line.starts_with("Usage:"))
                .unwrap();
            assert!(usage.matches("--output <OUTPUT>").count() <= 1, "{usage}");
        }
        assert_eq!(
            Args::try_parse_from(["yog", "--version"])
                .unwrap_err()
                .kind(),
            ErrorKind::DisplayVersion
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_paths_and_mode_like_file_names_are_preserved() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let native = OsString::from_vec(b"path-\xff".to_vec());
        let mut decoder = OsString::from("--decode-vaapi=");
        decoder.push(&native);
        let (request, _) = Args::try_parse_from([
            "yog".into(),
            native.clone(),
            "-o".into(),
            native.clone(),
            decoder,
            "--encode-vaapi".into(),
            "hevc".into(),
            "--device".into(),
            native.clone(),
        ])
        .unwrap()
        .into_request()
        .unwrap();
        assert_eq!(request.input.as_os_str(), native);
        assert_eq!(request.output.as_os_str(), native);
        assert!(
            matches!(request.decoding, DecodingBackend::Vaapi(Some(device)) if device == native)
        );
        assert!(
            matches!(request.video, VideoAction::Encode(VideoEncoding::Vaapi { device, .. }) if device.as_os_str() == native)
        );

        let (request, _) =
            Args::try_parse_from(["yog", "--output=--encode-vaapi", "--", "--encode-nvenc"])
                .unwrap()
                .into_request()
                .unwrap();
        assert_eq!(request.input, PathBuf::from("--encode-nvenc"));
        assert_eq!(request.output, PathBuf::from("--encode-vaapi"));
        assert!(matches!(request.video, VideoAction::Copy));
    }
}
