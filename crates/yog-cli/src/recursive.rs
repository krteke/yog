use crate::{diagnostics::Diagnostics, error::RunError, transcode::Transcoder};
use anyhow::Context;
use clap::ValueEnum;
use rustix::path::Arg;
use std::collections::HashSet;
use walkdir::WalkDir;
use yog_core::{
    ffmpeg::plan::{Container, TranscodeRequest},
    ffprobe::types::MediaInfo,
};

pub async fn discover(
    template: TranscodeRequest,
    transcoder: &Transcoder,
    diagnostics: &Diagnostics,
) -> Result<Vec<(TranscodeRequest, MediaInfo)>, RunError> {
    if !template.input.is_dir() {
        return Err(RunError::Failed(anyhow::anyhow!(
            "recursive input is not a directory: {}",
            template.input.display()
        )));
    }
    if template.output.exists() && !template.output.is_dir() {
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
                let mut warning = format!(
                    "warning: skipping {} because ffprobe failed: {error:#}\n",
                    input.display()
                );
                if let Some(error) = error
                    .chain()
                    .find_map(|cause| cause.downcast_ref::<yog_core::error::Error>())
                    && !error.stderr.is_empty()
                {
                    warning.push_str(&error.stderr.to_string_lossy());
                    if !warning.ends_with('\n') {
                        warning.push('\n');
                    }
                }
                diagnostics.write(warning.as_bytes());
                continue;
            }
        };
        if !media.streams.iter().any(|stream| stream.is_regular_video()) {
            continue;
        }

        let relative = input
            .strip_prefix(&template.input)
            .expect("walked entry must be below the input directory")
            .to_owned();
        let mut request = template.clone();
        request.input = input;
        let output_container = request
            .output_container(&media)
            .with_context(|| {
                format!(
                    "cannot select output container for {}",
                    request.input.display()
                )
            })
            .map_err(RunError::from)?;
        let input_container =
            Container::from_input(&request.input, media.format.format_name.as_deref());

        let mut output = template.output.join(relative);
        if request.container.is_some() && input_container != Some(output_container) {
            let value = output_container
                .to_possible_value()
                .expect("Container variants must have clap values");
            output.set_extension(value.get_name());
        }
        if !outputs.insert(output.clone()) {
            return Err(RunError::Failed(anyhow::anyhow!(
                "multiple input files map to {}",
                output.display()
            )));
        }
        request.output = output;
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
