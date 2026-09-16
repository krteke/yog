use yog_core::ffmpeg::{
    encoding::{RateControl, VideoEncoding},
    plan::VideoAction,
};

use crate::args::Args;

pub trait Validate {
    type Output;

    fn validate(self) -> anyhow::Result<Self::Output>;
}

impl Validate for Args {
    type Output = Self;

    fn validate(self) -> anyhow::Result<Self::Output> {
        let video = self.video.map(|video| video.validate()).transpose()?;
        let args = Self { video, ..self };

        if args.emulation.emulate {
            anyhow::ensure!(
                args.output.is_none(),
                "--output cannot be used with --emulate"
            );

            let Some(VideoAction::Encode(encoding)) = args.video.as_ref() else {
                anyhow::bail!("--emulate requires a video encoder");
            };

            anyhow::ensure!(
                encoding.rate().is_none(),
                "--quality and --bitrate cannot be used with --emulate"
            );
            anyhow::ensure!(
                args.emulation.png != args.emulation.svg,
                "--png and --svg must use different paths"
            );

            if let Some(range) = &args.emulation.range {
                let supported = encoding.quality_range();
                anyhow::ensure!(
                    supported.contains(range.start()) && supported.contains(range.end()),
                    "{} quality range must be within {}..={}, got {}..={}",
                    encoding.name(),
                    supported.start(),
                    supported.end(),
                    range.start(),
                    range.end(),
                );
            }
        } else if args.predict {
            anyhow::ensure!(
                args.output.is_none(),
                "--output cannot be used with --predict"
            );
        } else {
            anyhow::ensure!(args.output.is_some(), "--output is required");
        }

        Ok(args)
    }
}

impl Validate for VideoAction {
    type Output = Self;

    fn validate(self) -> anyhow::Result<Self::Output> {
        match self {
            VideoAction::Copy => Ok(VideoAction::Copy),
            VideoAction::Encode(encoding) => encoding.validate().map(VideoAction::Encode),
        }
    }
}

impl Validate for VideoEncoding {
    type Output = Self;

    fn validate(self) -> anyhow::Result<Self::Output> {
        let rate = self.rate();

        if let Some(RateControl::Quality(quality)) = rate {
            let range = self.quality_range();
            let encoder = self.name();
            let min = range.start();
            let max = range.end();

            anyhow::ensure!(
                range.contains(&quality),
                "{encoder} quality must be in {min}..={max}, got {quality}",
            );
        }

        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn validate(flags: &[&str]) -> anyhow::Result<Args> {
        Args::try_parse_from(
            ["yog", "input.mkv", "-o", "output.mkv"]
                .into_iter()
                .chain(flags.iter().copied()),
        )
        .unwrap()
        .validate()
    }

    #[test]
    fn quality_ranges_match_the_selected_ffmpeg_encoder() {
        for flags in [
            &["--encode-x264", "--quality", "51"][..],
            &["--encode-x265", "--quality", "51"],
            &["--encode-svt-av1", "--quality", "63"],
            &["--encode-aom-av1", "--quality", "63"],
            &["--encode-rav1e", "--quality", "255"],
            &["--encode-nvenc", "h264", "--quality", "51"],
            &["--encode-nvenc", "av1", "--quality", "63"],
            &["--encode-qsv", "hevc", "--quality", "1"],
            &["--encode-qsv", "av1", "--quality", "51"],
            &["--encode-vaapi", "h264", "--quality", "52"],
            &["--encode-vaapi", "av1", "--quality", "255"],
        ] {
            assert!(validate(flags).is_ok(), "{flags:?}");
        }

        for (flags, expected) in [
            (
                &["--encode-x264", "--quality", "52"][..],
                "libx264 quality must be in 0..=51, got 52",
            ),
            (
                &["--encode-x265", "--quality", "52"],
                "libx265 quality must be in 0..=51, got 52",
            ),
            (
                &["--encode-svt-av1", "--quality", "64"],
                "libsvtav1 quality must be in 0..=63, got 64",
            ),
            (
                &["--encode-aom-av1", "--quality", "64"],
                "libaom-av1 quality must be in 0..=63, got 64",
            ),
            (
                &["--encode-nvenc", "hevc", "--quality", "52"],
                "hevc_nvenc quality must be in 0..=51, got 52",
            ),
            (
                &["--encode-nvenc", "av1", "--quality", "64"],
                "av1_nvenc quality must be in 0..=63, got 64",
            ),
            (
                &["--encode-qsv", "h264", "--quality", "0"],
                "h264_qsv quality must be in 1..=51, got 0",
            ),
            (
                &["--encode-qsv", "av1", "--quality", "52"],
                "av1_qsv quality must be in 1..=51, got 52",
            ),
            (
                &["--encode-vaapi", "hevc", "--quality", "53"],
                "hevc_vaapi quality must be in 0..=52, got 53",
            ),
        ] {
            assert_eq!(
                validate(flags).unwrap_err().to_string(),
                expected,
                "{flags:?}"
            );
        }
    }
}
