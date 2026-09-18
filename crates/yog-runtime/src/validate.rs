use crate::{Command, Operation};
use yog_core::ffmpeg::{
    encoding::{RateControl, VideoEncoding},
    plan::VideoAction,
};

pub trait Validate {
    fn validate(&self) -> anyhow::Result<()>;
}

impl Validate for Command {
    fn validate(&self) -> anyhow::Result<()> {
        if let VideoAction::Encode(encoding) = &self.request.video {
            encoding.validate()?;
        }

        match &self.operation {
            Operation::Transcode => {
                anyhow::ensure!(
                    !self.request.output.as_os_str().is_empty(),
                    "transcoding requires an output path"
                );
            }
            Operation::Predict => {
                anyhow::ensure!(
                    self.request.output.as_os_str().is_empty(),
                    "prediction does not accept a video output"
                );
                anyhow::ensure!(
                    matches!(self.request.video, VideoAction::Encode(_)),
                    "prediction requires a video encoder"
                );
            }
            Operation::Emulate(options) => {
                anyhow::ensure!(
                    !self.recursive,
                    "emulation does not support recursive processing"
                );
                anyhow::ensure!(
                    self.request.output.as_os_str().is_empty(),
                    "emulation does not accept a video output"
                );
                anyhow::ensure!(
                    options.png.is_some() || options.svg.is_some(),
                    "emulation requires a PNG or SVG output"
                );
                anyhow::ensure!(
                    options.png.is_none() || options.png != options.svg,
                    "PNG and SVG outputs must use different paths"
                );

                let VideoAction::Encode(encoding) = &self.request.video else {
                    anyhow::bail!("emulation requires a video encoder");
                };
                anyhow::ensure!(
                    encoding.rate().is_none(),
                    "emulation does not accept a fixed quality or bitrate"
                );

                let supported = encoding.quality_range();
                if !options.qualities.is_empty() {
                    let start = *supported.start();
                    let end = *supported.end();
                    let qualities = options.qualities.as_slice();

                    anyhow::ensure!(
                        qualities[0] >= start && qualities[qualities.len() - 1] <= end,
                        "{} quality must be within {}..={}",
                        encoding.name(),
                        start,
                        end,
                    );
                }
            }
        }

        Ok(())
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

        Ok(())
    }
}
