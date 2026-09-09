use crate::{
    args::ExecutionOptions, diagnostics::Diagnostics, error::RunError, output::Output,
    progress::Display,
};
use anyhow::Context;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use yog_core::{
    ffmpeg::{Ffmpeg, plan::TranscodeRequest},
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

    let progress = Display::new(options.verbose);
    let timeout = options.timeout.map(Duration::from_secs);
    let target = request.output.clone();
    let output = Output::prepare(&target, request.overwrite)
        .with_context(|| format!("cannot prepare output {}", target.display()))?;

    request.output = output.part().to_owned();
    request.overwrite = true;

    let probe = Ffprobe::new(options.ffprobe, timeout).with_cancellation(cancelled.clone());
    let ffmpeg = Ffmpeg::new(options.ffmpeg, timeout).with_cancellation(cancelled.clone());
    let media = probe
        .probe(&request.input)
        .await
        .context("probe failed")?
        .output;

    let plan = request
        .plan(&media, &ffmpeg)
        .await
        .context("cannot plan strict pixel formats")?;
    progress.start(media.format.duration.as_deref());
    ffmpeg
        .execute(
            plan.args(),
            |record| progress.update(record),
            |bytes| diagnostics.ffmpeg(bytes),
        )
        .await
        .context("transcode failed")?;

    if cancelled.is_cancelled() {
        return Err(RunError::Cancelled);
    }

    progress.publishing();
    output
        .publish()
        .with_context(|| format!("cannot publish output {}", target.display()))?;

    Ok(())
}
