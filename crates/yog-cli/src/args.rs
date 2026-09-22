use clap::{ArgGroup, CommandFactory, Parser, Subcommand, error::ErrorKind};
use std::{num::NonZeroU32, path::PathBuf, str::FromStr, time::Duration};
use yog_core::ffmpeg::{
    decoding::DecodingBackend,
    encoding::VideoEncoding,
    plan::{Container, TranscodeRequest, VideoAction},
    vmaf::VmafOptions,
};
use yog_runtime::{Candidate, Command, EmulationOptions, Operation, Options, parse_quality_points};

use crate::decoding::DecodingArgs;

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Args {
    #[arg(short, long, global = true, value_name = "PATH")]
    input: Option<PathBuf>,
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
    #[arg(short, long, global = true, value_name = "PATH")]
    output: Option<PathBuf>,
    #[arg(short, long, global = true)]
    recursive: bool,
    #[arg(short = 'O', long, global = true)]
    overwrite: bool,
    #[arg(short, long, global = true)]
    verify: bool,
    #[arg(long, global = true, value_name = "PATH")]
    report: Option<PathBuf>,
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
    #[arg(long, global = true, value_name = "PATH")]
    report: Option<PathBuf>,
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
    #[command(flatten)]
    video: VideoArgs,
    #[arg(long, group = "emulation_output", value_name = "PATH")]
    png: Option<PathBuf>,
    #[arg(long, group = "emulation_output", value_name = "PATH")]
    svg: Option<PathBuf>,
    /// e.g. --candidate vaapi:libsvtav1:7:2-40
    #[arg(
        long,
        value_parser = parse_candidate,
        value_name = "DECODE:ENCODE:PRESET:RANGE[:MULTIPASS]"
    )]
    candidate: Vec<Candidate>,
    #[arg(long, global = true, value_name = "PATH")]
    report: Option<PathBuf>,
    #[arg(short = 'O', long, global = true)]
    overwrite: bool,
}

#[derive(Debug, clap::Args)]
struct VideoArgs {
    /// e.g. --range=20,25,30-35
    #[arg(long, value_parser = parse_quality_points, global = true, value_name = "POINTS")]
    range: Vec<Vec<u8>>,
    #[command(subcommand)]
    encoding: Option<VideoEncoding>,
}

fn parse_candidate(value: &str) -> Result<Candidate, String> {
    let segments = value.split(':').collect::<Vec<_>>();
    let (decoding, encoding, preset, range, multipass) = match segments.as_slice() {
        [decoding, encoding, preset, range] => (*decoding, *encoding, *preset, *range, None),
        [decoding, encoding, preset, range, multipass] => {
            (*decoding, *encoding, *preset, *range, Some(*multipass))
        }
        _ => {
            return Err(format!(
                "expected DECODE:ENCODE:PRESET:RANGE[:MULTIPASS], got {value}"
            ));
        }
    };

    let decoding = decoding.parse::<DecodingBackend>()?;
    let mut encoding = encoding.parse::<VideoEncoding>()?;
    encoding.try_set_preset(preset)?;
    let qualities = parse_quality_points(range)?;

    match (&mut encoding, multipass) {
        (VideoEncoding::Nvenc { multipass, .. }, Some(value)) => {
            *multipass = Some(value.parse()?);
        }
        (VideoEncoding::Nvenc { .. }, None) | (_, None) => {}
        (_, Some(_)) => return Err("multipass is only supported by NVENC".to_owned()),
    }

    Ok(Candidate {
        decoding,
        encoding,
        qualities,
    })
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

        let input = input.ok_or_else(|| missing_argument("--input <PATH>"))?;
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
            report: None,
        };
        let (operation, mut request, recursive) = match command {
            CliCommand::Transcode(args) => {
                let output = args
                    .output
                    .ok_or_else(|| missing_argument("--output <PATH>"))?;
                options.verify = args.verify;
                options.vmaf = args.vmaf.map(Into::into);
                options.report = args.report;
                (
                    Operation::Transcode,
                    TranscodeRequest::new(input, output)
                        .with_video(args.video.unwrap_or_default())
                        .with_overwrite(args.overwrite),
                    args.recursive,
                )
            }
            CliCommand::Predict(args) => {
                options.report = args.report;
                (
                    Operation::Predict,
                    TranscodeRequest::new(input, PathBuf::new())
                        .with_video(VideoAction::Encode(args.video)),
                    args.recursive,
                )
            }
            CliCommand::Emulate(args) => {
                options.report = args.report;
                let mut qualities = args.video.range.into_iter().flatten().collect::<Vec<_>>();
                qualities.sort_unstable();
                qualities.dedup();
                (
                    Operation::Emulate(EmulationOptions {
                        png: args.png,
                        svg: args.svg,
                        qualities,
                        candidates: args.candidate,
                    }),
                    TranscodeRequest::new(input, PathBuf::new())
                        .with_video(
                            args.video
                                .encoding
                                .map_or(VideoAction::Copy, VideoAction::Encode),
                        )
                        .with_overwrite(args.overwrite),
                    false,
                )
            }
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

fn missing_argument(argument: &str) -> clap::Error {
    clap::Error::raw(
        ErrorKind::MissingRequiredArgument,
        format!("the following required argument was not provided: {argument}"),
    )
    .format(&mut Args::command())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yog_core::ffmpeg::{
        decoding::DecodingBackend,
        encoding::{RateControl, VideoCodec},
    };

    fn parse<const N: usize>(args: [&str; N]) -> (Option<PathBuf>, Command, Options) {
        Args::try_parse_from(args).unwrap().into_runtime().unwrap()
    }

    #[track_caller]
    fn assert_parse_error(args: Vec<&str>, kind: ErrorKind) {
        let error = Args::try_parse_from(args).unwrap_err();
        assert_eq!(error.kind(), kind, "{error}");
    }

    #[track_caller]
    fn assert_conversion_error(args: Vec<&str>, kind: ErrorKind) {
        let error = Args::try_parse_from(args)
            .unwrap()
            .into_runtime()
            .unwrap_err();
        assert_eq!(error.kind(), kind, "{error}");
    }

    #[track_caller]
    fn assert_candidate(
        value: &str,
        decoding: &str,
        encoder: &str,
        preset: Option<&str>,
        multipass: Option<&str>,
        qualities: &[u8],
    ) {
        let candidate = parse_candidate(value).unwrap();
        assert_eq!(candidate.decoding.to_string(), decoding);
        assert_eq!(candidate.encoding.name(), encoder);
        assert_eq!(candidate.encoding.preset().as_deref(), preset);
        assert_eq!(candidate.encoding.multipass(), multipass);
        assert_eq!(candidate.qualities, qualities);
    }

    #[track_caller]
    fn assert_invalid_quality_range(range: &str) {
        let range = format!("--range={range}");
        assert_parse_error(
            vec![
                "yog",
                "-i",
                "input",
                "emulate",
                "--png",
                "plot.png",
                &range,
                "--encode-x264",
            ],
            ErrorKind::ValueValidation,
        );
    }

    #[test]
    fn command_tree_scopes_operation_arguments() {
        Args::command().debug_assert();

        assert_conversion_error(
            vec!["yog", "transcode", "-o", "output", "--copy"],
            ErrorKind::MissingRequiredArgument,
        );
        assert_conversion_error(
            vec!["yog", "-i", "input", "transcode", "--copy"],
            ErrorKind::MissingRequiredArgument,
        );
        assert_parse_error(
            vec!["yog", "-i", "input", "emulate", "--encode-x264"],
            ErrorKind::MissingRequiredArgument,
        );
        assert_parse_error(
            vec!["yog", "-i", "input", "predict", "--verify", "--encode-x264"],
            ErrorKind::UnknownArgument,
        );
        assert_parse_error(
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
        );
        assert_parse_error(
            vec!["yog", "-i", "input", "predict", "--copy"],
            ErrorKind::UnknownArgument,
        );
        assert_parse_error(
            vec!["yog", "-i", "input", "--predict", "--encode-x264"],
            ErrorKind::UnknownArgument,
        );
    }

    #[test]
    fn reports_are_available_to_every_operation() {
        let (_, _, transcode) = parse([
            "yog",
            "-i",
            "input",
            "transcode",
            "-o",
            "output",
            "--copy",
            "--report",
            "report.jsonl",
        ]);
        let (_, _, predict) = parse([
            "yog",
            "-i",
            "input",
            "predict",
            "--encode-x264",
            "--report",
            "report.jsonl",
        ]);
        let (_, _, recursive) = parse([
            "yog",
            "-i",
            "input",
            "predict",
            "--recursive",
            "--report",
            "report.jsonl",
            "--encode-x264",
        ]);
        let (_, _, emulate) = parse([
            "yog",
            "-i",
            "input",
            "emulate",
            "--png",
            "plot.png",
            "--report",
            "report.jsonl",
            "--encode-x264",
        ]);

        let expected = Some(PathBuf::from("report.jsonl"));
        assert_eq!(transcode.report, expected);
        assert_eq!(predict.report, expected);
        assert_eq!(recursive.report, expected);
        assert_eq!(emulate.report, expected);
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
        assert!(options.report.is_none());

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
        assert!(options.report.is_none());

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
            "20-22",
            "--overwrite",
            "--encode-x264",
        ]);
        assert!(emulation.request.output.as_os_str().is_empty());
        assert!(emulation.request.overwrite);
        assert!(matches!(
            &emulation.operation,
            Operation::Emulate(EmulationOptions { qualities, .. }) if qualities == &vec![20, 21, 22]
        ));
    }

    #[test]
    fn quality_points_expand_merge_sort_and_deduplicate() {
        assert_eq!(qualities(["--range", ""]), Vec::<u8>::new());
        assert_eq!(qualities(["--range", "  "]), Vec::<u8>::new());
        assert_eq!(qualities(["--range", "20"]), vec![20]);
        assert_eq!(qualities(["--range", "20,22"]), vec![20, 22]);
        assert_eq!(qualities(["--range", "20-22"]), vec![20, 21, 22]);
        assert_eq!(qualities(["--range", "20-22,25"]), vec![20, 21, 22, 25]);
        assert_eq!(
            qualities(["--range", "20-25,23-30"]),
            (20..=30).collect::<Vec<u8>>()
        );
        assert_eq!(qualities(["--range", "30,10-12,30"]), vec![10, 11, 12, 30]);
        assert_eq!(qualities(["--range", "20-20"]), vec![20]);
        assert_eq!(
            qualities(["--range", " 20 - 22 , 25 "]),
            vec![20, 21, 22, 25]
        );
        assert_eq!(
            qualities(["--range", "0-255"]),
            (0..=255).collect::<Vec<u8>>()
        );
        assert_eq!(
            qualities(["--range", "18-20", "--range", "25,20"]),
            vec![18, 19, 20, 25],
        );
    }

    #[test]
    fn quality_points_reject_malformed_ranges() {
        assert_invalid_quality_range("-");
        assert_invalid_quality_range("20-");
        assert_invalid_quality_range("-20");
        assert_invalid_quality_range("20-22-24");
        assert_invalid_quality_range("20,,25");
        assert_invalid_quality_range("256");
        assert_invalid_quality_range("20,256");
        assert_invalid_quality_range("30-20");
        assert_invalid_quality_range("20-20-21");
    }

    #[test]
    fn candidates_carry_decoder_encoder_knobs_and_points() {
        assert_candidate(
            "vaapi:libsvtav1:7:2-4",
            "vaapi",
            "libsvtav1",
            Some("7"),
            None,
            &[2, 3, 4],
        );
        assert_candidate(
            "vaapi=/dev/dri/renderD129:hevc_nvenc:p5:20-21:qres",
            "vaapi:/dev/dri/renderD129",
            "hevc_nvenc",
            Some("p5"),
            Some("qres"),
            &[20, 21],
        );
        assert_candidate(
            "software:libx264:medium:0,3",
            "software",
            "libx264",
            Some("medium"),
            None,
            &[0, 3],
        );
        assert_candidate(
            ":hevc_nvenc::20-22",
            "software",
            "hevc_nvenc",
            None,
            None,
            &[20, 21, 22],
        );
        assert_candidate(
            "cuda=0:av1_qsv:slow:5",
            "cuda:0",
            "av1_qsv",
            Some("slow"),
            None,
            &[5],
        );
        assert_candidate(
            "qsv=1:libaom-av1:6:7",
            "qsv:1",
            "libaom-av1",
            Some("6"),
            None,
            &[7],
        );

        let candidate = parse_candidate("software:h264_vaapi=/dev/dri/renderD129::5").unwrap();
        assert!(
            matches!(
                candidate.encoding,
                VideoEncoding::Vaapi { ref device, .. }
                    if device.to_string_lossy() == "/dev/dri/renderD129"
            ),
            "{candidate:?}"
        );
    }

    #[test]
    fn candidate_parser_rejects_ignored_or_extra_fields() {
        assert!(parse_candidate("software:libx264:medium:20:garbage").is_err());
        assert!(parse_candidate("software:libx264:medium:20:garbage:extra").is_err());
        assert!(parse_candidate("software:libx264=/dev/dri/renderD128:medium:20").is_err());
        assert!(parse_candidate("software:libx264=:medium:20").is_err());
        assert!(parse_candidate("software:h264_nvenc=1:p5:20").is_err());
        assert!(parse_candidate("software=:libx264:medium:20").is_err());
        assert!(parse_candidate("software:libx264:medium:20:qres").is_err());
    }

    #[test]
    fn candidate_and_range_are_preserved_for_runtime_validation() {
        let (_, command, _) = Args::try_parse_from([
            "yog",
            "-i",
            "input",
            "emulate",
            "--png",
            "plot.png",
            "--range",
            "20-22",
            "--candidate",
            "software:libx264:medium:20-21",
        ])
        .unwrap()
        .into_runtime()
        .unwrap();

        let Operation::Emulate(options) = command.operation else {
            panic!("expected emulation")
        };
        assert_eq!(options.qualities, vec![20, 21, 22]);
        assert_eq!(options.candidates.len(), 1);
        assert_eq!(options.candidates[0].qualities, vec![20, 21]);
    }

    fn qualities<const N: usize>(range: [&str; N]) -> Vec<u8> {
        let mut args = vec!["yog", "-i", "input", "emulate", "--png", "plot.png"];
        args.extend(range);
        args.push("--encode-x264");
        let (_, command, _) = Args::try_parse_from(args).unwrap().into_runtime().unwrap();
        match command.operation {
            Operation::Emulate(EmulationOptions { qualities, .. }) => qualities,
            operation => panic!("expected emulation, got {operation:?}"),
        }
    }

    #[test]
    fn vmaf_rejects_zero_at_the_cli_boundary() {
        assert_parse_error(
            vec![
                "yog",
                "-i",
                "input",
                "transcode",
                "-o",
                "output",
                "--vmaf=0",
            ],
            ErrorKind::ValueValidation,
        );
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
        assert!(emulate.contains("--report"));
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
