use super::RateControl;
use clap::{ArgMatches, Args, Command, FromArgMatches, error::ErrorKind};
use std::num::NonZeroU64;

#[derive(Args)]
#[group(id = "rate", multiple = false)]
struct RateArgs {
    #[arg(long, short)]
    quality: Option<u8>,
    #[arg(long, short)]
    bitrate: Option<NonZeroU64>,
}

impl Args for RateControl {
    fn group_id() -> Option<clap::Id> {
        RateArgs::group_id()
    }

    fn augment_args(command: Command) -> Command {
        RateArgs::augment_args(command)
    }

    fn augment_args_for_update(command: Command) -> Command {
        RateArgs::augment_args_for_update(command)
    }
}

impl FromArgMatches for RateControl {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        let args = RateArgs::from_arg_matches(matches)?;
        args.quality
            .map(Self::Quality)
            .or_else(|| args.bitrate.map(Self::Bitrate))
            .ok_or_else(|| {
                clap::Error::raw(
                    ErrorKind::MissingRequiredArgument,
                    "needs --quality or --bitrate",
                )
            })
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        if matches.contains_id("rate") {
            *self = Self::from_arg_matches(matches)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ffmpeg::plan::{TranscodeRequest, VideoAction},
        ffprobe::types::MediaInfo,
    };
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        video: VideoAction,
    }

    #[test]
    fn backend_specific_types_reach_the_transcode_plan() {
        let media: MediaInfo =
            serde_json::from_str(r#"{"streams":[{"index":0,"codec_type":"video"}]}"#).unwrap();
        for (flags, expected) in [
            (
                vec!["--encode-x265", "--preset", "slow", "--bitrate", "4000000"],
                vec![
                    ("-c:v", "libx265"),
                    ("-preset:v", "slow"),
                    ("-b:v", "4000000"),
                ],
            ),
            (
                vec!["--encode-svt-av1", "--preset", "9", "--quality", "30"],
                vec![("-c:v", "libsvtav1"), ("-preset:v", "9"), ("-crf:v", "30")],
            ),
            (
                vec!["--encode-aom-av1", "--cpu-used", "6", "--quality", "31"],
                vec![
                    ("-c:v", "libaom-av1"),
                    ("-cpu-used:v", "6"),
                    ("-crf:v", "31"),
                ],
            ),
            (
                vec!["--encode-rav1e", "--speed", "7", "--quality", "90"],
                vec![("-c:v", "librav1e"), ("-speed:v", "7"), ("-qp:v", "90")],
            ),
            (
                vec![
                    "--encode-qsv",
                    "hevc",
                    "--preset",
                    "veryfast",
                    "--quality",
                    "27",
                ],
                vec![
                    ("-c:v", "hevc_qsv"),
                    ("-preset:v", "veryfast"),
                    ("-global_quality:v", "27"),
                ],
            ),
            (
                vec![
                    "--encode-vaapi",
                    "av1",
                    "--device",
                    "/dev/dri/custom",
                    "--quality",
                    "28",
                ],
                vec![
                    ("-c:v", "av1_vaapi"),
                    ("-vaapi_device", "/dev/dri/custom"),
                    ("-qp:v", "28"),
                ],
            ),
        ] {
            let video = Cli::try_parse_from(["yog"].into_iter().chain(flags.iter().copied()))
                .unwrap()
                .video;
            let plan = TranscodeRequest::mkv("input", "output")
                .with_video(video)
                .plan(&media);
            for (option, value) in expected {
                assert!(
                    plan.args()
                        .windows(2)
                        .any(|pair| pair[0] == option && pair[1] == value),
                    "{flags:?}: missing {option} {value} in {:?}",
                    plan.args()
                );
            }
        }
    }
}
