use crate::{diagnostics::Diagnostics, error::RunError, transcode::Transcoder};
use anyhow::Context;
use std::collections::HashSet;
use walkdir::WalkDir;
use yog_core::{ffmpeg::plan::TranscodeRequest, ffprobe::types::MediaInfo};

pub async fn discover(
    template: TranscodeRequest,
    transcoder: &Transcoder,
    diagnostics: &Diagnostics,
    predict: bool,
) -> Result<Vec<(TranscodeRequest, MediaInfo)>, RunError> {
    if !template.input.is_dir() {
        return Err(RunError::Failed(anyhow::anyhow!(
            "recursive input is not a directory: {}",
            template.input.display()
        )));
    }
    if !predict && template.output.exists() && !template.output.is_dir() {
        return Err(RunError::Failed(anyhow::anyhow!(
            "recursive output is not a directory: {}",
            template.output.display()
        )));
    }

    let mut tasks = Vec::new();
    let mut outputs = HashSet::new();
    for entry in WalkDir::new(&template.input) {
        let entry = entry
            .with_context(|| format!("cannot read input directory {}", template.input.display()))
            .map_err(RunError::from)?;
        if !entry.file_type().is_file() {
            continue;
        }

        let input = entry.into_path();
        let media = match transcoder.probe(&input).await {
            Ok(media) => media,
            Err(RunError::Cancelled) => return Err(RunError::Cancelled),
            Err(RunError::Failed(error)) => {
                diagnostics.skipped_probe(&input, &error);
                continue;
            }
        };
        if !media.streams.iter().any(|stream| stream.is_regular_video()) {
            continue;
        }

        let mut request = template.clone();
        request.input = input;
        if !predict {
            let relative = request
                .input
                .strip_prefix(&template.input)
                .expect("walked entry must be below the input directory");
            let output_container = request
                .output_container(&media)
                .with_context(|| {
                    format!(
                        "cannot select output container for {}",
                        request.input.display()
                    )
                })
                .map_err(RunError::from)?;
            let mut output = template.output.join(relative);
            if request.container.is_some() {
                output.set_extension(output_container.extension());
            }
            if !outputs.insert(output.clone()) {
                return Err(RunError::Failed(anyhow::anyhow!(
                    "multiple input files map to {}",
                    output.display()
                )));
            }
            request.output = output;
        }
        tasks.push((request, media));
    }

    if tasks.is_empty() {
        return Err(RunError::Failed(anyhow::anyhow!(
            "no video files found in {}",
            template.input.display()
        )));
    }
    Ok(tasks)
}
