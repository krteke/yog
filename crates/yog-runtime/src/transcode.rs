use crate::{
    Config, Options,
    diagnostics::Diagnostics,
    error::RunError,
    output::Output,
    progress::Display,
    report::{
        record::{PredictRecord, TranscodeRecord, VerifyOutcome, VerifyRecord, VmafRecord},
        transcode::TranscodeResult,
    },
    verify::{VerificationOutcome, Verifier},
};
use anyhow::Context;
use rustix::path::Arg;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use yog_core::{
    ffmpeg::{
        Ffmpeg,
        plan::{TranscodeRequest, VideoAction},
        prediction::{Prediction, PredictionOptions},
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
    pub(super) config: Config,
    pub(super) verbose: bool,
    terminal_output: bool,
}

impl Transcoder {
    pub fn new(options: &Options, config: Config, cancelled: CancellationToken) -> Self {
        Self {
            probe: Ffprobe::new(&options.ffprobe, options.timeout)
                .with_cancellation(cancelled.clone())
                .with_data_hashes(options.verify),
            ffmpeg: Ffmpeg::new(&options.ffmpeg, options.timeout).with_cancellation(cancelled),
            verify: options.verify,
            vmaf: options.vmaf,
            config,
            verbose: options.verbose,
            terminal_output: options.terminal_output,
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
        record: &mut TranscodeRecord,
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
        record.fill_source(&request, &media);
        self.execute(request, media, output, diagnostics, record)
            .await
    }

    pub async fn run_probed(
        &self,
        request: TranscodeRequest,
        media: MediaInfo,
        diagnostics: &Diagnostics,
        record: &mut TranscodeRecord,
    ) -> Result<(), RunError> {
        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        let output = Output::prepare(&request.output, request.overwrite)
            .with_context(|| format!("cannot prepare output {}", request.output.display()))?;
        record.fill_source(&request, &media);
        self.execute(request, media, output, diagnostics, record)
            .await
    }

    pub async fn predict(
        &self,
        request: TranscodeRequest,
        diagnostics: &Diagnostics,
        record: &mut PredictRecord,
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
        self.predict_probed(request, media, diagnostics, record)
            .await
    }

    pub async fn predict_probed(
        &self,
        request: TranscodeRequest,
        media: MediaInfo,
        diagnostics: &Diagnostics,
        record: &mut PredictRecord,
    ) -> Result<(), RunError> {
        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        record.fill_source(&request, &media);
        let options = self.prediction_options();
        let progress = Display::predicting(self.progress_visible(), self.tick_interval());

        let result = self
            .predict_result(&request, &media, options, diagnostics)
            .await;

        drop(progress);
        let prediction = match result {
            Ok(prediction) => prediction,
            Err(error) => return Err(RunError::from(error.context("prediction failed"))),
        };
        diagnostics.predict(&request.input, &prediction);
        record.fill_prediction(&prediction);

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

    pub(super) fn prediction_options(&self) -> PredictionOptions {
        self.config.prediction.into()
    }

    pub(super) fn cancelled(&self) -> bool {
        self.ffmpeg.cancellation().is_cancelled()
    }

    pub(super) fn progress_visible(&self) -> bool {
        self.terminal_output && !self.verbose
    }

    pub(super) fn tick_interval(&self) -> Duration {
        Duration::from_millis(self.config.progress_tick_interval_ms.get())
    }

    async fn execute(
        &self,
        mut request: TranscodeRequest,
        media: MediaInfo,
        output: Output,
        diagnostics: &Diagnostics,
        record: &mut TranscodeRecord,
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

        let progress = Display::new(self.progress_visible(), self.tick_interval());
        progress.start(media.format.try_duration().ok());
        let started = Instant::now();
        let mut reported = None;
        let result = command
            .run(
                |record| {
                    progress.update(record);
                    reported = Some(record);
                },
                |bytes| {
                    diagnostics.ffmpeg(bytes);
                },
            )
            .await;
        let elapsed = started.elapsed().as_secs_f64();
        drop(progress);
        result.context("transcode failed")?;

        let mut warnings = Vec::new();
        let mut verification = None;
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
                Ok(VerificationOutcome::Complete) => {
                    verification = Some(VerifyOutcome::Complete);
                }
                Ok(VerificationOutcome::MissingAudioAfterSeek {
                    source_index,
                    destination_index,
                    timestamp,
                }) => {
                    record.fill_verify(VerifyRecord::new(
                        VerifyOutcome::MissingAudioAfterSeek,
                        warnings,
                    ));
                    return Err(RunError::Failed(anyhow::anyhow!(
                        "verification failed: audio stream #{source_index} -> #{destination_index} has no decoded frames after seeking to {timestamp}s"
                    )));
                }
                Err(error) => {
                    if matches!(error.reason, yog_core::error::Failure::Cancelled) {
                        return Err(RunError::Cancelled);
                    }
                    verification = Some(VerifyOutcome::Incomplete);
                    warnings.push(format!("verification incomplete: {error}"));
                    if !error.stderr.is_empty() {
                        warnings.push(error.stderr.to_string_lossy().trim_end().to_owned());
                    }
                }
            }
        }

        for warning in &warnings {
            diagnostics.verify_warning(warning);
        }
        if let Some(verification) = verification {
            record.fill_verify(VerifyRecord::new(verification, warnings));
        }

        if self.cancelled() {
            return Err(RunError::Cancelled);
        }

        output
            .publish()
            .with_context(|| format!("cannot publish output {}", target.display()))?;
        diagnostics.complete(&target);

        let processed_seconds = reported
            .as_ref()
            .and_then(|update| update.out_time_us)
            .map(|microseconds| microseconds as f64 / 1_000_000.0);
        let media_seconds = processed_seconds.or_else(|| {
            media
                .format
                .try_duration()
                .ok()
                .map(|duration| duration.as_secs_f64())
        });
        let bytes = fs::metadata(&target).ok().map(|metadata| metadata.len());
        record.fill_result(TranscodeResult::new(
            bytes,
            processed_seconds,
            elapsed,
            media_seconds.and_then(|seconds| (elapsed > 0.0).then(|| seconds / elapsed)),
            reported.as_ref().and_then(|update| update.frame),
            reported.as_ref().and_then(|update| update.fps),
        ));

        if let Some(options) = self.vmaf {
            let progress = Display::calculating_vmaf(self.progress_visible(), self.tick_interval());
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
                Ok(score) => {
                    record.fill_vmaf(VmafRecord::requested(options).scored(score.value));
                    diagnostics.vmaf(&target, score, options);
                }
                Err(error) => match RunError::from(error) {
                    RunError::Cancelled => return Err(RunError::Cancelled),
                    RunError::Failed(error) => {
                        record
                            .fill_vmaf(VmafRecord::requested(options).failed(format!("{error:#}")));
                        diagnostics.vmaf_warning(&error, stderr_logged);
                    }
                },
            }
        }

        Ok(())
    }
}
