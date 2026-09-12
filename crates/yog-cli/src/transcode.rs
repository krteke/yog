use crate::{
    args::ExecutionOptions,
    diagnostics::Diagnostics,
    error::RunError,
    output::Output,
    progress::Display,
    verify::{VerificationOutcome, Verifier},
};
use anyhow::Context;
use rustix::path::Arg;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use yog_core::{
    ffmpeg::{
        Ffmpeg,
        plan::{TranscodeRequest, VideoAction},
    },
    ffprobe::Ffprobe,
};

pub async fn run(
    mut request: TranscodeRequest,
    options: ExecutionOptions,
    cancelled: CancellationToken,
    diagnostics: &Diagnostics,
) -> Result<(), RunError> {
    if cancelled.is_cancelled() {
        return Err(RunError::Cancelled);
    }

    if !request.input.exists() {
        return Err(RunError::Failed(anyhow::anyhow!(
            "input file does not exist"
        )));
    }

    let timeout = options.timeout.map(Duration::from_secs);
    let target = request.output.clone();
    let output = Output::prepare(&target, request.overwrite)
        .with_context(|| format!("cannot prepare output {}", target.display()))?;

    request.output = output.part().to_owned();
    request.overwrite = true;

    let probe = Ffprobe::new(options.ffprobe, timeout)
        .with_cancellation(cancelled.clone())
        .with_data_hashes(options.verify);
    let ffmpeg = Ffmpeg::new(options.ffmpeg, timeout).with_cancellation(cancelled.clone());
    let media = probe
        .probe(&request.input)
        .await
        .context("probe failed")?
        .output;

    let plan = request
        .plan(&media, &ffmpeg)
        .await
        .context("cannot plan transcode")?;
    let command = ffmpeg
        .build(&plan)
        .context("cannot build transcode command")?;
    command.print();

    let progress = Display::new(options.verbose);
    progress.start(media.format.duration.as_deref());
    command
        .run(
            |record| progress.update(record),
            |bytes| diagnostics.ffmpeg(bytes),
        )
        .await
        .context("transcode failed")?;
    drop(progress);

    let mut warnings = Vec::new();
    let verifier = Verifier {
        probe: &probe,
        ffmpeg: &ffmpeg,
    };

    if options.verify {
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

    if cancelled.is_cancelled() {
        return Err(RunError::Cancelled);
    }

    let published = output
        .publish()
        .with_context(|| format!("cannot publish output {}", target.display()));

    for warning in warnings {
        diagnostics.write(format!("warning: verify: {warning}\n").as_bytes());
    }
    published?;

    Ok(())
}
