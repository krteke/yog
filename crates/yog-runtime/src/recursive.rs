use crate::{
    error::{RunError, timed_out},
    event::{RunPhase, SkipReason, TaskResult},
    recovery::RecoveredTask,
    transcode::Transcoder,
};
use anyhow::Context;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::PathBuf,
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;
use yog_core::{ffmpeg::plan::TranscodeRequest, ffprobe::types::MediaInfo};

pub struct TaskPaths {
    pub relative: PathBuf,
    pub input: PathBuf,
    pub output: PathBuf,
}

pub fn scan(
    template: TranscodeRequest,
    cancellation: CancellationToken,
    predict: bool,
) -> Result<Vec<TaskPaths>, RunError> {
    if cancellation.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    if !template.input.is_dir() {
        return Err(RunError::at(
            RunPhase::Discovering,
            anyhow::anyhow!(
                "recursive input is not a directory: {}",
                template.input.display()
            ),
        ));
    }
    if !predict && template.output.exists() && !template.output.is_dir() {
        return Err(RunError::at(
            RunPhase::Discovering,
            anyhow::anyhow!(
                "recursive output is not a directory: {}",
                template.output.display()
            ),
        ));
    }

    let mut tasks = Vec::new();
    for entry in WalkDir::new(&template.input) {
        if cancellation.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let entry = entry
            .with_context(|| format!("cannot read input directory {}", template.input.display()))
            .map_err(|error| RunError::at(RunPhase::Discovering, error))?;
        if !entry.file_type().is_file() {
            continue;
        }

        let input = entry.into_path();
        let relative = input
            .strip_prefix(&template.input)
            .expect("walked entry belongs to input")
            .to_path_buf();
        let output = if predict {
            PathBuf::new()
        } else {
            let mut output = template.output.join(&relative);
            if let Some(container) = template.container {
                output.set_extension(container.extension());
            }
            output
        };
        tasks.push(TaskPaths {
            relative,
            input,
            output,
        });
    }

    Ok(tasks)
}

/// Probe only possible collisions, so same-named non-video sidecars do not
/// prevent a batch. Media information is discarded before probing the next file.
pub async fn check_output_conflicts(
    tasks: &[TaskPaths],
    template: &TranscodeRequest,
    transcoder: &Transcoder,
    recovered: &HashMap<usize, RecoveredTask>,
    skipped: &mut HashMap<usize, TaskResult>,
) -> Result<(), RunError> {
    let mut candidates = HashMap::new();
    let mut conflicts = BTreeSet::new();
    for (index, task) in tasks.iter().enumerate() {
        if recovered.get(&index).is_some_and(|task| {
            !matches!(
                task.result,
                TaskResult::Skipped {
                    reason: SkipReason::AlreadyCompleted,
                    ..
                }
            )
        }) {
            continue;
        }
        if let Some(previous) = candidates.insert(&task.output, index) {
            conflicts.insert(previous);
            conflicts.insert(index);
        }
    }
    drop(candidates);

    let mut outputs = HashSet::new();
    for index in conflicts {
        let task = &tasks[index];
        if recovered.contains_key(&index) {
            if !outputs.insert(&task.output) {
                return Err(RunError::at(
                    RunPhase::Discovering,
                    anyhow::anyhow!("multiple input files map to {}", task.output.display()),
                ));
            }
            continue;
        }
        let request = TranscodeRequest {
            input: task.input.clone(),
            output: task.output.clone(),
            ..template.clone()
        };
        match probe_task(&request, transcoder, false).await {
            Ok(_) => {
                if !outputs.insert(&task.output) {
                    return Err(RunError::at(
                        RunPhase::Discovering,
                        anyhow::anyhow!("multiple input files map to {}", task.output.display()),
                    ));
                }
            }
            Err(RunError::Cancelled) => return Err(RunError::Cancelled),
            Err(error @ RunError::Skipped { .. }) => {
                skipped.insert(index, error.result());
            }
            Err(RunError::Failed { .. }) => unreachable!("batch probing returns skips"),
        }
    }
    Ok(())
}

pub async fn probe_task(
    request: &TranscodeRequest,
    transcoder: &Transcoder,
    predict: bool,
) -> Result<MediaInfo, RunError> {
    let media = match transcoder.probe(&request.input).await {
        Ok(media) => media,
        Err(RunError::Cancelled) => return Err(RunError::Cancelled),
        Err(RunError::Failed { error, .. }) => {
            return Err(RunError::Skipped {
                reason: if timed_out(&error) {
                    SkipReason::ProbeTimedOut
                } else {
                    SkipReason::ProbeFailed
                },
                error,
            });
        }
        Err(RunError::Skipped { .. }) => unreachable!("raw probes do not decide eligibility"),
    };
    if !media.streams.iter().any(|stream| stream.is_regular_video()) {
        return Err(RunError::Skipped {
            reason: SkipReason::NoVideo,
            error: anyhow::anyhow!("no regular video streams"),
        });
    }
    if !predict {
        request
            .output_container(&media)
            .with_context(|| {
                format!(
                    "cannot select output container for {}",
                    request.input.display()
                )
            })
            .map_err(|error| RunError::Skipped {
                reason: SkipReason::UnsupportedContainer,
                error,
            })?;
    }
    Ok(media)
}
