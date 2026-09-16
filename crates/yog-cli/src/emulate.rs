mod chart;

use crate::{
    config, diagnostics::Diagnostics, error::RunError, progress::Display, transcode::Transcoder,
};
use anyhow::Context;
use std::{ops::RangeInclusive, path::PathBuf};
use yog_core::ffmpeg::plan::{TranscodeRequest, VideoAction};

pub struct EmulationOptions {
    pub png: Option<PathBuf>,
    pub svg: Option<PathBuf>,
    pub qualities: RangeInclusive<u8>,
}

struct EmulationPoint {
    quality: u8,
    vmaf: f64,
    size_bytes: u64,
}

impl Transcoder {
    pub async fn emulate(
        &self,
        request: TranscodeRequest,
        options: &EmulationOptions,
        diagnostics: &Diagnostics,
    ) -> Result<(), RunError> {
        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        if !request.input.exists() {
            return Err(RunError::Failed(anyhow::anyhow!(
                "input file does not exist"
            )));
        }

        chart::check_paths(options, request.overwrite)
            .context("cannot prepare emulation chart output")?;
        let media = self.probe(&request.input).await?;
        let prediction_options = config::get().prediction.into();
        let progress = Display::emulating(self.verbose);
        let maximum = *options.qualities.end();
        let VideoAction::Encode(encoding) = &request.video else {
            unreachable!("emulation arguments require a video encoder");
        };
        let quality_parameter = encoding.quality_parameter();
        let mut points = Vec::with_capacity(options.qualities.clone().count());

        for quality in options.qualities.clone() {
            if self.cancelled() {
                return Err(RunError::Cancelled);
            }
            progress.emulate_quality(quality_parameter, quality, maximum);
            let mut sample_request = request.clone();
            let VideoAction::Encode(encoding) = &mut sample_request.video else {
                unreachable!("emulation arguments require a video encoder");
            };
            encoding.set_quality(quality);
            let prediction = self
                .predict_result(&sample_request, &media, prediction_options, diagnostics)
                .await
                .with_context(|| format!("prediction failed at {quality_parameter} {quality}"))?;
            points.push(EmulationPoint {
                quality,
                vmaf: prediction.quality.vmaf.value,
                size_bytes: prediction.output_bytes.value,
            });
        }
        drop(progress);

        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        chart::render(&request, options, &points).context("cannot render emulation chart")?;
        diagnostics.emulation(&request.input, quality_parameter, options);
        Ok(())
    }
}
