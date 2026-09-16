use crate::{
    args::ExecutionOptions,
    config,
    diagnostics::Diagnostics,
    error::RunError,
    output::Output,
    progress::Display,
    verify::{VerificationOutcome, Verifier},
};
use anyhow::Context;
use rustix::path::Arg;
use std::{path::Path, time::Duration};
use tokio_util::sync::CancellationToken;
use yog_core::{
    ffmpeg::{
        Ffmpeg,
        plan::{TranscodeRequest, VideoAction},
        prediction::Prediction,
        vmaf::VmafOptions,
    },
    ffprobe::Ffprobe,
    ffprobe::types::MediaInfo,
};

pub struct Transcoder {
    probe: Ffprobe,
    ffmpeg: Ffmpeg,
    verify: bool,
    vmaf: Option<VmafOptions>,
    pub(super) verbose: bool,
}

impl Transcoder {
    pub fn new(options: &ExecutionOptions, cancelled: CancellationToken) -> Self {
        let timeout = options.timeout.map(Duration::from_secs);
        Self {
            probe: Ffprobe::new(&options.ffprobe, timeout)
                .with_cancellation(cancelled.clone())
                .with_data_hashes(options.verify),
            ffmpeg: Ffmpeg::new(&options.ffmpeg, timeout).with_cancellation(cancelled),
            verify: options.verify,
            vmaf: options.vmaf.map(Into::into),
            verbose: options.verbose,
        }
    }

    pub async fn probe(&self, input: &Path) -> Result<MediaInfo, RunError> {
        self.probe
            .probe(input)
            .await
            .context("probe failed")
            .map(|result| result.output)
            .map_err(RunError::from)
    }

    pub async fn run(
        &self,
        request: TranscodeRequest,
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

        let output = Output::prepare(&request.output, request.overwrite)
            .with_context(|| format!("cannot prepare output {}", request.output.display()))?;
        let media = self.probe(&request.input).await?;
        self.execute(request, media, output, diagnostics).await
    }

    pub async fn run_probed(
        &self,
        request: TranscodeRequest,
        media: MediaInfo,
        diagnostics: &Diagnostics,
    ) -> Result<(), RunError> {
        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        let output = Output::prepare(&request.output, request.overwrite)
            .with_context(|| format!("cannot prepare output {}", request.output.display()))?;
        self.execute(request, media, output, diagnostics).await
    }

    pub async fn predict(
        &self,
        request: TranscodeRequest,
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
        let media = self.probe(&request.input).await?;
        self.predict_probed(request, media, diagnostics).await
    }

    pub async fn predict_probed(
        &self,
        request: TranscodeRequest,
        media: MediaInfo,
        diagnostics: &Diagnostics,
    ) -> Result<(), RunError> {
        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        let config = config::get();
        let options = config.prediction.into();
        let progress = Display::predicting(self.verbose);

        let result = self
            .predict_result(&request, &media, options, diagnostics)
            .await
            .context("prediction failed")?;

        drop(progress);
        diagnostics.predict(&request.input, &result);

        Ok(())
    }

    pub(super) async fn predict_result(
        &self,
        request: &TranscodeRequest,
        media: &MediaInfo,
        options: yog_core::ffmpeg::prediction::PredictionOptions,
        diagnostics: &Diagnostics,
    ) -> anyhow::Result<Prediction> {
        self.ffmpeg
            .predict(&self.probe, request, media, options, |bytes| {
                diagnostics.ffmpeg(bytes);
            })
            .await
            .map_err(|e| e.into())
    }

    pub(super) fn cancelled(&self) -> bool {
        self.ffmpeg.cancellation().is_cancelled()
    }

    async fn execute(
        &self,
        mut request: TranscodeRequest,
        media: MediaInfo,
        output: Output,
        diagnostics: &Diagnostics,
    ) -> Result<(), RunError> {
        let target = request.output.clone();
        request.output = output.part().to_owned();
        request.overwrite = true;

        let plan = request
            .plan(&media, &self.ffmpeg)
            .await
            .context("cannot plan transcode")?;
        let command = self
            .ffmpeg
            .build(&plan)
            .context("cannot build transcode command")?;

        let progress = Display::new(self.verbose);
        progress.start(media.format.try_duration().ok());
        command
            .run(
                |record| progress.update(record),
                |bytes| {
                    diagnostics.ffmpeg(bytes);
                },
            )
            .await
            .context("transcode failed")?;
        drop(progress);

        let mut warnings = Vec::new();
        let verifier = Verifier {
            probe: &self.probe,
            ffmpeg: &self.ffmpeg,
        };

        if self.verify {
            match verifier
                .verify(
                    &request.input,
                    output.part(),
                    &media,
                    matches!(request.video, VideoAction::Copy),
                    &plan,
                    |warning| warnings.push(warning),
                )
                .await
            {
                Ok(VerificationOutcome::Complete) => {}
                Ok(VerificationOutcome::MissingAudioAfterSeek {
                    source_index,
                    destination_index,
                    timestamp,
                }) => {
                    return Err(RunError::Failed(anyhow::anyhow!(
                        "verification failed: audio stream #{source_index} -> #{destination_index} has no decoded frames after seeking to {timestamp}s"
                    )));
                }
                Err(error) => {
                    if matches!(error.reason, yog_core::error::Failure::Cancelled) {
                        return Err(RunError::Cancelled);
                    }
                    warnings.push(format!("verification incomplete: {error}"));
                    if !error.stderr.is_empty() {
                        warnings.push(error.stderr.to_string_lossy().trim_end().to_owned());
                    }
                }
            }
        }

        for warning in warnings {
            eprintln!("warning: verify: {warning}");
        }

        if self.cancelled() {
            return Err(RunError::Cancelled);
        }

        output
            .publish()
            .with_context(|| format!("cannot publish output {}", target.display()))?;
        println!("complete: {}", target.display());

        if let Some(options) = self.vmaf {
            let progress = Display::calculating_vmaf(self.verbose);
            let mut stderr_logged = false;
            let result = self
                .ffmpeg
                .vmaf(
                    &self.probe,
                    &request.input,
                    &media,
                    &target,
                    options,
                    |bytes| {
                        stderr_logged |= diagnostics.ffmpeg(bytes);
                    },
                )
                .await
                .context("VMAF calculation failed");
            drop(progress);
            match result {
                Ok(score) => diagnostics.vmaf(&target, score, options),
                Err(error) => match RunError::from(error) {
                    RunError::Cancelled => return Err(RunError::Cancelled),
                    RunError::Failed(error) => diagnostics.vmaf_warning(&error, stderr_logged),
                },
            }
        }

        Ok(())
    }
}
