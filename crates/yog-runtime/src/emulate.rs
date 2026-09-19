mod chart;

use crate::{
    TaskOutcome,
    diagnostics::Diagnostics,
    error::RunError,
    progress::Display,
    record_task,
    report::{Report, record::EmulateRecord},
    transcode::Transcoder,
};
use anyhow::Context;
use std::{ops::RangeInclusive, path::PathBuf};
use yog_core::ffmpeg::plan::{TranscodeRequest, VideoAction};

#[derive(Debug, Clone)]
pub struct EmulationOptions {
    pub png: Option<PathBuf>,
    pub svg: Option<PathBuf>,
    pub qualities: Vec<u8>,
}

impl EmulationOptions {
    fn select_qualities(&self, full: RangeInclusive<u8>) -> Vec<u8> {
        if self.qualities.is_empty() {
            return full.collect();
        }

        self.qualities.clone()
    }
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
        report: &mut Report,
    ) -> Result<(), RunError> {
        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        if !request.input.exists() {
            return Err(RunError::Failed(anyhow::anyhow!(
                "input file does not exist"
            )));
        }
        let source_bytes = request
            .input
            .metadata()
            .with_context(|| format!("cannot read input metadata {}", request.input.display()))?
            .len();

        chart::check_paths(options, request.overwrite)
            .context("cannot prepare emulation chart output")?;
        let media = self.probe(&request.input).await?;
        let prediction_options = self.config.prediction.into();
        let progress = Display::emulating(self.progress_visible(), self.tick_interval());
        let VideoAction::Encode(encoding) = &request.video else {
            unreachable!("emulation arguments require a video encoder");
        };
        let qualities = options.select_qualities(encoding.quality_range());

        let quality_parameter = encoding.quality_parameter();
        let mut points = Vec::with_capacity(qualities.len());

        for (index, &quality) in qualities.iter().enumerate() {
            if self.cancelled() {
                return Err(RunError::Cancelled);
            }
            progress.emulate_quality(quality_parameter, quality, index + 1, qualities.len());
            let mut sample_request = request.clone();
            let VideoAction::Encode(encoding) = &mut sample_request.video else {
                unreachable!("emulation arguments require a video encoder");
            };
            encoding.set_quality(quality);

            let mut record = EmulateRecord::new(
                &sample_request,
                quality,
                options.png.as_deref(),
                options.svg.as_deref(),
                prediction_options,
            );
            record.fill_source(&sample_request, &media);
            let result = self
                .predict_result(&sample_request, &media, prediction_options, diagnostics)
                .await;
            let outcome = match result {
                Ok(prediction) => {
                    record.fill_prediction(&prediction);
                    points.push(EmulationPoint {
                        quality,
                        vmaf: prediction.quality.vmaf.value,
                        size_bytes: prediction.output_bytes.value,
                    });
                    TaskOutcome::Success
                }
                Err(error) => error
                    .context(format!(
                        "prediction failed at {quality_parameter} {quality}"
                    ))
                    .into(),
            };
            record_task(record, &outcome, report, diagnostics);

            match outcome {
                TaskOutcome::Success => {}
                TaskOutcome::Failed(error) => return Err(RunError::Failed(error)),
                TaskOutcome::Cancelled => return Err(RunError::Cancelled),
            }
        }
        drop(progress);

        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        chart::render(
            &request,
            options,
            &points,
            source_bytes,
            (
                self.config.emulation.width.get(),
                self.config.emulation.height.get(),
            ),
        )
        .context("cannot render emulation chart")?;
        diagnostics.emulation(&request.input, quality_parameter, &qualities, options);
        Ok(())
    }
}
