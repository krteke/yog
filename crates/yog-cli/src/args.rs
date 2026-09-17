use clap::{ArgGroup, CommandFactory, Parser, Subcommand};
use std::{num::NonZeroU32, ops::RangeInclusive, path::PathBuf, str::FromStr, time::Duration};
use yog_core::ffmpeg::{
    encoding::VideoEncoding,
    plan::{Container, TranscodeRequest, VideoAction},
    vmaf::VmafOptions,
};
use yog_runtime::{Command, EmulationOptions, Operation, Options};

use crate::decoding::DecodingArgs;

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Args {
    #[arg(short, long, global = true, required = false, value_name = "PATH")]
    input: PathBuf,
    #[arg(short, long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[arg(short = 'C', long, global = true)]
    container: Option<Container>,
    #[command(flatten)]
    decoding: DecodingArgs,
    #[command(flatten)]
    execution: ExecutionOptions,
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Transcode(TranscodeArgs),
    Predict(PredictArgs),
    Emulate(EmulateArgs),
}

#[derive(Debug, clap::Args)]
struct TranscodeArgs {
    #[arg(short, long, global = true, required = false, value_name = "PATH")]
    output: PathBuf,
    #[arg(short, long, global = true)]
    recursive: bool,
    #[arg(short = 'O', long, global = true)]
    overwrite: bool,
    #[arg(short, long, global = true)]
    verify: bool,
    #[arg(
        long,
        global = true,
        value_name = "full|N_SUBSAMPLE",
        num_args = 0..=1,
        default_missing_value = "full",
        require_equals = true
    )]
    vmaf: Option<VmafMode>,
    #[command(subcommand)]
    video: Option<VideoAction>,
}

#[derive(Debug, clap::Args)]
struct PredictArgs {
    #[arg(short, long, global = true)]
    recursive: bool,
    #[command(subcommand)]
    video: VideoEncoding,
}

#[derive(Debug, clap::Args)]
#[command(group(
    ArgGroup::new("emulation_output")
        .args(["png", "svg"])
        .required(true)
        .multiple(true)
))]
struct EmulateArgs {
    #[arg(long, group = "emulation_output", value_name = "PATH")]
    png: Option<PathBuf>,
    #[arg(long, group = "emulation_output", value_name = "PATH")]
    svg: Option<PathBuf>,
    #[arg(long, value_parser = parse_quality_range, value_name = "MIN,MAX")]
    range: Option<RangeInclusive<u8>>,
    #[arg(short = 'O', long, global = true)]
    overwrite: bool,
    #[command(subcommand)]
    video: VideoEncoding,
}

#[derive(Debug, clap::Args)]
struct ExecutionOptions {
    #[arg(long, default_value = "ffmpeg", global = true)]
    ffmpeg: PathBuf,
    #[arg(long, default_value = "ffprobe", global = true)]
    ffprobe: PathBuf,
    #[arg(long, short, global = true, value_name = "SECONDS")]
    timeout: Option<u64>,
    #[arg(long, global = true)]
    verbose: bool,
    #[arg(long, global = true, conflicts_with = "verbose")]
    quiet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VmafMode {
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
    pub(super) fn into_runtime(self) -> Result<(Option<PathBuf>, Command, Options), clap::Error> {
        let Self {
            input,
            config,
            container,
            decoding,
            execution,
            command,
        } = self;

        let decoding = decoding
            .try_into()
            .map_err(|error: clap::Error| error.format(&mut Self::command()))?;

        let mut options = Options {
            ffmpeg: execution.ffmpeg,
            ffprobe: execution.ffprobe,
            timeout: execution.timeout.map(Duration::from_secs),
            verbose: execution.verbose,
            verify: false,
            vmaf: None,
            terminal_output: !execution.quiet,
        };
        let (operation, mut request, recursive) = match command {
            CliCommand::Transcode(args) => {
                options.verify = args.verify;
                options.vmaf = args.vmaf.map(Into::into);
                (
                    Operation::Transcode,
                    TranscodeRequest::new(input, args.output)
                        .with_video(args.video.unwrap_or_default())
                        .with_overwrite(args.overwrite),
                    args.recursive,
                )
            }
            CliCommand::Predict(args) => (
                Operation::Predict,
                TranscodeRequest::new(input, PathBuf::new())
                    .with_video(VideoAction::Encode(args.video)),
                args.recursive,
            ),
            CliCommand::Emulate(args) => (
                Operation::Emulate(EmulationOptions {
                    png: args.png,
                    svg: args.svg,
                    qualities: args.range,
                }),
                TranscodeRequest::new(input, PathBuf::new())
                    .with_video(VideoAction::Encode(args.video))
                    .with_overwrite(args.overwrite),
                false,
            ),
        };
        request = request.with_decoding(decoding);
        if let Some(container) = container {
            request = request.with_container(container);
        }

        Ok((
            config,
            Command {
                request,
                operation,
                recursive,
            },
            options,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use yog_core::ffmpeg::{
        decoding::DecodingBackend,
        encoding::{RateControl, VideoCodec},
    };
    use yog_runtime::Validate;

    fn parse<const N: usize>(args: [&str; N]) -> (Option<PathBuf>, Command, Options) {
        Args::try_parse_from(args).unwrap().into_runtime().unwrap()
    }

    #[test]
    fn command_tree_scopes_operation_arguments() {
        Args::command().debug_assert();

        let missing_input = Args::try_parse_from(["yog", "transcode", "-o", "output"])
            .unwrap()
            .into_runtime()
            .unwrap_err();
        assert_eq!(missing_input.kind(), ErrorKind::MissingRequiredArgument);
        assert!(
            missing_input
                .to_string()
                .contains("--input <PATH> is required")
        );

        for (args, kind) in [
            (
                vec!["yog", "-i", "input", "transcode", "--copy"],
                ErrorKind::MissingRequiredArgument,
            ),
            (
                vec!["yog", "-i", "input", "predict", "--verify", "--encode-x264"],
                ErrorKind::UnknownArgument,
            ),
            (
                vec![
                    "yog",
                    "-i",
                    "input",
                    "emulate",
                    "--recursive",
                    "--png",
                    "plot.png",
                    "--encode-x264",
                ],
                ErrorKind::UnknownArgument,
            ),
            (
                vec!["yog", "-i", "input", "emulate", "--encode-x264"],
                ErrorKind::MissingRequiredArgument,
            ),
            (
                vec!["yog", "-i", "input", "predict", "--copy"],
                ErrorKind::UnknownArgument,
            ),
            (
                vec!["yog", "-i", "input", "--predict", "--encode-x264"],
                ErrorKind::UnknownArgument,
            ),
        ] {
            let error = Args::try_parse_from(args).unwrap_err();
            assert_eq!(error.kind(), kind, "{error}");
        }
    }

    #[test]
    fn global_arguments_work_before_between_and_after_nested_subcommands() {
        let (_, transcode, options) = parse([
            "yog",
            "transcode",
            "-o",
            "output",
            "--copy",
            "-i",
            "input",
            "-C",
            "mp4",
            "--decode-cuda=0",
            "--timeout",
            "7",
            "--verbose",
            "--ffmpeg",
            "custom-ffmpeg",
            "--ffprobe",
            "custom-ffprobe",
        ]);
        assert_eq!(transcode.request.input, PathBuf::from("input"));
        assert_eq!(transcode.request.output, PathBuf::from("output"));
        assert_eq!(transcode.request.container, Some(Container::Mp4));
        assert!(matches!(transcode.request.video, VideoAction::Copy));
        assert!(matches!(
            transcode.request.decoding,
            DecodingBackend::Cuda(Some(ref device)) if device == "0"
        ));
        assert_eq!(options.timeout, Some(Duration::from_secs(7)));
        assert!(options.verbose);
        assert_eq!(options.ffmpeg, PathBuf::from("custom-ffmpeg"));
        assert_eq!(options.ffprobe, PathBuf::from("custom-ffprobe"));

        let (_, predict, _) = parse([
            "yog",
            "-C",
            "webm",
            "predict",
            "--recursive",
            "--encode-nvenc",
            "hevc",
            "--quality",
            "27",
            "--input",
            "input",
        ]);
        assert_eq!(predict.request.input, PathBuf::from("input"));
        assert_eq!(predict.request.container, Some(Container::Webm));
        assert!(predict.recursive);
        assert!(matches!(
            predict.request.video,
            VideoAction::Encode(VideoEncoding::Nvenc {
                codec: VideoCodec::Hevc,
                rate: Some(RateControl::Quality(27)),
                ..
            })
        ));

        let (_, emulate, _) = parse([
            "yog",
            "emulate",
            "--png",
            "quality.png",
            "--encode-x264",
            "--input",
            "input",
        ]);
        assert_eq!(emulate.request.input, PathBuf::from("input"));
        assert!(matches!(emulate.operation, Operation::Emulate(_)));
    }

    #[test]
    fn operations_map_directly_to_runtime_commands() {
        let (_, transcode, options) = parse([
            "yog",
            "-i",
            "input",
            "transcode",
            "-o",
            "output",
            "--recursive",
            "--overwrite",
            "--verify",
            "--vmaf=7",
        ]);
        assert!(matches!(transcode.operation, Operation::Transcode));
        assert!(matches!(transcode.request.video, VideoAction::Copy));
        assert!(transcode.request.overwrite);
        assert!(transcode.recursive);
        assert!(options.verify);
        assert_eq!(options.vmaf.unwrap().n_subsample, NonZeroU32::new(7));

        let (_, prediction, options) = parse([
            "yog",
            "predict",
            "--recursive",
            "--encode-x264",
            "--quality",
            "23",
            "-i",
            "input",
        ]);
        assert!(matches!(prediction.operation, Operation::Predict));
        assert!(prediction.request.output.as_os_str().is_empty());
        assert!(prediction.recursive);
        assert!(!options.verify);
        assert!(options.vmaf.is_none());

        let (_, emulation, _) = parse([
            "yog",
            "-i",
            "input",
            "emulate",
            "--png",
            "quality.png",
            "--svg",
            "quality.svg",
            "--range",
            "20,22",
            "--overwrite",
            "--encode-x264",
        ]);
        emulation.validate().unwrap();
        assert!(emulation.request.output.as_os_str().is_empty());
        assert!(emulation.request.overwrite);
        assert!(matches!(
            emulation.operation,
            Operation::Emulate(EmulationOptions {
                qualities: Some(ref range),
                ..
            }) if range == &(20..=22)
        ));
    }

    #[test]
    fn vmaf_and_emulation_ranges_reject_invalid_values() {
        for args in [
            vec![
                "yog",
                "-i",
                "input",
                "transcode",
                "-o",
                "output",
                "--vmaf=0",
            ],
            vec![
                "yog",
                "-i",
                "input",
                "emulate",
                "--png",
                "plot.png",
                "--range",
                "30,20",
                "--encode-x264",
            ],
            vec![
                "yog",
                "-i",
                "input",
                "emulate",
                "--png",
                "plot.png",
                "--range",
                "256,257",
                "--encode-x264",
            ],
        ] {
            assert!(Args::try_parse_from(args).is_err());
        }

        for (args, expected) in [
            (
                vec![
                    "yog",
                    "-i",
                    "input",
                    "emulate",
                    "--png",
                    "plot.png",
                    "--range",
                    "50,52",
                    "--encode-x264",
                ],
                "libx264 quality range must be within 0..=51, got 50..=52",
            ),
            (
                vec![
                    "yog",
                    "-i",
                    "input",
                    "emulate",
                    "--png",
                    "plot.png",
                    "--encode-x264",
                    "--quality",
                    "20",
                ],
                "emulation does not accept a fixed quality or bitrate",
            ),
            (
                vec![
                    "yog",
                    "-i",
                    "input",
                    "emulate",
                    "--png",
                    "plot",
                    "--svg",
                    "plot",
                    "--encode-x264",
                ],
                "PNG and SVG outputs must use different paths",
            ),
        ] {
            let (_, command, _) = Args::try_parse_from(args).unwrap().into_runtime().unwrap();
            assert_eq!(command.validate().unwrap_err().to_string(), expected);
        }
    }

    #[test]
    fn help_follows_operation_then_encoder_hierarchy() {
        let root = Args::try_parse_from(["yog", "--help"]).unwrap_err();
        assert_eq!(root.kind(), ErrorKind::DisplayHelp);
        let root = root.to_string();
        assert!(root.contains("transcode"));
        assert!(root.contains("predict"));
        assert!(root.contains("emulate"));
        assert!(!root.contains("--png"));
        assert!(!root.contains("--output"));

        let predict = Args::try_parse_from(["yog", "predict", "--help"]).unwrap_err();
        assert_eq!(predict.kind(), ErrorKind::DisplayHelp);
        let predict = predict.to_string();
        assert!(predict.contains("--encode-x264"));
        assert!(predict.contains("--recursive"));
        assert!(!predict.contains("--output"));
        assert!(!predict.contains("--vmaf"));

        let emulate = Args::try_parse_from(["yog", "emulate", "--help"]).unwrap_err();
        assert_eq!(emulate.kind(), ErrorKind::DisplayHelp);
        let emulate = emulate.to_string();
        assert!(emulate.contains("--png"));
        assert!(emulate.contains("--range"));
        assert!(!emulate.contains("--recursive"));

        let encoder =
            Args::try_parse_from(["yog", "transcode", "--encode-nvenc", "--help"]).unwrap_err();
        assert_eq!(encoder.kind(), ErrorKind::DisplayHelp);
        let encoder = encoder.to_string();
        assert!(encoder.contains("--multipass"));
        assert!(encoder.contains("--input"));
        assert!(!encoder.contains("--device"));
    }

    #[cfg(unix)]
    #[test]
    fn native_and_option_like_paths_are_preserved() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let native = OsString::from_vec(b"path-\xff".to_vec());
        let mut decoder = OsString::from("--decode-vaapi=");
        decoder.push(&native);
        let (_, command, _) = Args::try_parse_from([
            "yog".into(),
            "transcode".into(),
            "-o".into(),
            native.clone(),
            "--encode-vaapi".into(),
            "hevc".into(),
            "--device".into(),
            native.clone(),
            "--input".into(),
            native.clone(),
            decoder,
        ])
        .unwrap()
        .into_runtime()
        .unwrap();
        assert_eq!(command.request.input.as_os_str(), native);
        assert_eq!(command.request.output.as_os_str(), native);
        assert!(
            matches!(command.request.decoding, DecodingBackend::Vaapi(Some(device)) if device == native)
        );
        assert!(
            matches!(command.request.video, VideoAction::Encode(VideoEncoding::Vaapi { device, .. }) if device.as_os_str() == native)
        );

        let (_, command, _) = Args::try_parse_from([
            "yog",
            "transcode",
            "--output=--encode-vaapi",
            "--copy",
            "--input=--encode-nvenc",
        ])
        .unwrap()
        .into_runtime()
        .unwrap();
        assert_eq!(command.request.input, PathBuf::from("--encode-nvenc"));
        assert_eq!(command.request.output, PathBuf::from("--encode-vaapi"));
    }
}
