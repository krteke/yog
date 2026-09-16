use crate::{Command, Operation};
use yog_core::ffmpeg::{
    encoding::{RateControl, VideoEncoding},
    plan::VideoAction,
};

pub(super) fn command(command: &Command) -> anyhow::Result<()> {
    if let VideoAction::Encode(encoding) = &command.request.video {
        encoding_quality(encoding)?;
    }

    match &command.operation {
        Operation::Transcode => {
            anyhow::ensure!(
                !command.request.output.as_os_str().is_empty(),
                "--output is required"
            );
        }
        Operation::Predict => {
            anyhow::ensure!(
                command.request.output.as_os_str().is_empty(),
                "--output cannot be used with --predict"
            );
        }
        Operation::Emulate(options) => {
            anyhow::ensure!(
                !command.recursive,
                "--emulate cannot be used with --recursive"
            );
            anyhow::ensure!(
                command.request.output.as_os_str().is_empty(),
                "--output cannot be used with --emulate"
            );
            anyhow::ensure!(
                options.png.is_some() || options.svg.is_some(),
                "--emulate requires --png or --svg"
            );
            anyhow::ensure!(
                options.png.is_none() || options.png != options.svg,
                "--png and --svg must use different paths"
            );

            let VideoAction::Encode(encoding) = &command.request.video else {
                anyhow::bail!("--emulate requires a video encoder");
            };
            anyhow::ensure!(
                encoding.rate().is_none(),
                "--quality and --bitrate cannot be used with --emulate"
            );

            if let Some(range) = &options.qualities {
                anyhow::ensure!(range.start() <= range.end(), "MIN must not exceed MAX");
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
        }
    }

    Ok(())
}

fn encoding_quality(encoding: &VideoEncoding) -> anyhow::Result<()> {
    if let Some(RateControl::Quality(quality)) = encoding.rate() {
        let range = encoding.quality_range();
        anyhow::ensure!(
            range.contains(&quality),
            "{} quality must be in {}..={}, got {quality}",
            encoding.name(),
            range.start(),
            range.end(),
        );
    }

    Ok(())
}
