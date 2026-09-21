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
    use crate::ffmpeg::{
        args::{Arg, ArgsExt},
        encoding::VideoEncoding,
        plan::VideoAction,
    };
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        video: VideoAction,
    }

    fn encoder_options(flags: &[&str]) -> Vec<String> {
        let video = Cli::try_parse_from(["yog"].into_iter().chain(flags.iter().copied()))
            .unwrap()
            .video;
        let VideoAction::Encode(encoding) = video else {
            panic!("expected encoder")
        };
        let mut args = Vec::new();
        args.add(Arg::VideoCodec(encoding.name()));
        encoding.append_options(&mut args);
        if let VideoEncoding::Vaapi { device, .. } = &encoding {
            args.add(Arg::VaapiDevice(device));
        }
        args.into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect()
    }

    #[test]
    fn x265_cli_values_reach_encoder_options() {
        assert_eq!(
            encoder_options(&["--encode-x265", "--preset", "slow", "--bitrate", "4000000"]),
            ["-c:v", "libx265", "-b:v", "4000000", "-preset:v", "slow"]
        );
    }

    #[test]
    fn svt_av1_cli_values_reach_encoder_options() {
        assert_eq!(
            encoder_options(&["--encode-svt-av1", "--preset", "9", "--quality", "30"]),
            ["-c:v", "libsvtav1", "-crf:v", "30", "-preset:v", "9"]
        );
    }

    #[test]
    fn aom_av1_cli_values_reach_encoder_options() {
        assert_eq!(
            encoder_options(&["--encode-aom-av1", "--cpu-used", "6", "--quality", "31"]),
            [
                "-c:v",
                "libaom-av1",
                "-b:v",
                "0",
                "-crf:v",
                "31",
                "-cpu-used:v",
                "6"
            ]
        );
    }

    #[test]
    fn rav1e_cli_values_reach_encoder_options() {
        assert_eq!(
            encoder_options(&["--encode-rav1e", "--speed", "7", "--quality", "90"]),
            ["-c:v", "librav1e", "-qp:v", "90", "-speed:v", "7"]
        );
    }

    #[test]
    fn qsv_cli_values_reach_encoder_options() {
        assert_eq!(
            encoder_options(&[
                "--encode-qsv",
                "hevc",
                "--preset",
                "veryfast",
                "--quality",
                "27",
            ]),
            [
                "-c:v",
                "hevc_qsv",
                "-global_quality:v",
                "27",
                "-preset:v",
                "veryfast"
            ]
        );
    }

    #[test]
    fn vaapi_cli_values_reach_encoder_options() {
        assert_eq!(
            encoder_options(&[
                "--encode-vaapi",
                "av1",
                "--device",
                "/dev/dri/custom",
                "--quality",
                "28",
            ]),
            [
                "-c:v",
                "av1_vaapi",
                "-rc_mode:v",
                "CQP",
                "-global_quality:v",
                "28",
                "-vaapi_device",
                "/dev/dri/custom"
            ]
        );
    }
}
