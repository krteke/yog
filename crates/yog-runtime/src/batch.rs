use crate::{
    BatchOutcome, Command, Operation, Options, RunOutcome, TaskFailure, TaskOutcome,
    control::RunControl,
    diagnostics::Diagnostics,
    error::RunError,
    event::{RunEvent, RunPhase, SkipReason, TaskResult},
    output::Output,
    path::Absolute,
    recovery::{Recovery, RecoveryMode},
    recursive,
    report::{
        Record, Report,
        record::{PredictRecord, SkippedRecord, TaskRecord, TranscodeRecord},
    },
    transcode::Transcoder,
};
use std::{collections::HashMap, path::Path};
use yog_core::ffmpeg::plan::TranscodeRequest;

pub async fn run(
    command: Command,
    transcoder: &mut Transcoder,
    diagnostics: &Diagnostics<'_>,
    control: &RunControl,
    options: &Options,
) -> RunOutcome {
    let cancellation = control.cancellation();
    if cancellation.is_cancelled() {
        diagnostics.cancelled();
        return RunOutcome::Cancelled;
    }
    diagnostics.event(RunEvent::PhaseChanged(RunPhase::Discovering));

    let prepared = async {
        let predict = matches!(command.operation, Operation::Predict);
        let recovery = if matches!(command.operation, Operation::Transcode)
            && options.recovery.mode != RecoveryMode::Off
        {
            Some(Recovery::open(&command, options, diagnostics)?)
        } else {
            None
        };
        let scan_template = command.request.clone();
        let scan_cancellation = cancellation.clone();
        let tasks = tokio::task::spawn_blocking(move || {
            recursive::scan(scan_template, scan_cancellation, predict)
        })
        .await
        .expect("batch path scan panicked")
        .map_err(|error| match error {
            RunError::Failed { error, .. } => error,
            RunError::Cancelled => anyhow::anyhow!("batch cancelled"),
            RunError::Skipped { .. } => unreachable!("scans do not probe files"),
        })?;

        Ok::<_, anyhow::Error>((tasks, recovery))
    }
    .await;

    let (tasks, mut recovery) = match prepared {
        Ok(prepared) => prepared,
        Err(_) if cancellation.is_cancelled() => {
            diagnostics.cancelled();
            return RunOutcome::Cancelled;
        }
        Err(error) => {
            diagnostics.error(&error);
            return RunOutcome::Failed(error);
        }
    };

    let total = tasks.len();
    diagnostics.event(RunEvent::BatchDiscovered { total });
    if let Some(state) = recovery.as_mut()
        && let Err(error) = state.classify(&tasks, &cancellation, diagnostics).await
    {
        let mut batch = BatchOutcome {
            total,
            cancelled: cancellation.is_cancelled(),
            ..BatchOutcome::default()
        };
        if !batch.cancelled {
            diagnostics.error(&error);
            batch.recovery_error = Some(error);
        }
        diagnostics.batch_summary(&batch);
        return RunOutcome::Batch(batch);
    }

    let empty_recovered = HashMap::new();
    let recovered = recovery
        .as_ref()
        .map_or(&empty_recovered, Recovery::recovered);
    let predict = matches!(command.operation, Operation::Predict);
    let mut preflight_skipped = HashMap::new();
    if !predict && command.request.container.is_some() {
        match recursive::check_output_conflicts(
            &tasks,
            &command.request,
            transcoder,
            recovered,
            &mut preflight_skipped,
        )
        .await
        {
            Ok(_) | Err(RunError::Cancelled) => {}
            Err(RunError::Failed { error, .. }) => {
                diagnostics.error(&error);
                return RunOutcome::Failed(error);
            }
            Err(RunError::Skipped { .. }) => unreachable!("preflight consumes skips"),
        }
    }

    let report = (|| {
        if let Some(report) = &options.report {
            let report = report.try_absolutize()?;
            if !predict {
                let output_root = command.request.output.try_absolutize()?;
                if tasks.iter().any(|task| {
                    let mut output = output_root.join(&task.relative);
                    if let Some(container) = command.request.container {
                        output.set_extension(container.extension());
                    }
                    output == report
                        || Output::part_path(&output).expect("mapped output has filename") == report
                }) {
                    anyhow::bail!("report path conflicts with a mapped video output");
                }
            }
        }
        Report::open(options.report.as_deref(), options.overwrite_report)
    })();

    let mut report = match report {
        Ok(report) => report,
        Err(error) => {
            diagnostics.error(&error);
            return RunOutcome::Failed(error);
        }
    };

    let mut batch = BatchOutcome {
        total,
        ..BatchOutcome::default()
    };
    if let Some(state) = recovery.as_mut()
        && !cancellation.is_cancelled()
        && let Err(error) = state.start(diagnostics)
    {
        diagnostics.error(&error);
        batch.state_error = Some(error);
        diagnostics.batch_summary(&batch);
        return RunOutcome::Batch(batch);
    }

    let mut recovered = recovery
        .as_mut()
        .map_or(empty_recovered, Recovery::take_recovered);

    for (offset, task) in tasks.into_iter().enumerate() {
        if cancellation.is_cancelled() {
            break;
        }
        let restored = recovered.remove(&offset);
        let preflight = preflight_skipped.remove(&offset);
        let manual = !predict && restored.is_none() && preflight.is_none() && control.take_skip();
        let preparation_error = if restored.is_none() && !manual {
            let error = recovery
                .as_mut()
                .and_then(|state| state.begin(offset, &task).err());
            if recovery.as_ref().is_some_and(Recovery::has_error) {
                break;
            }
            error
        } else {
            None
        };
        let (skip, info) = if let Some(restored) = restored {
            (Some(restored.result), Some(restored.info))
        } else {
            (
                if manual {
                    if let Some(state) = recovery.as_mut() {
                        state.skip_manual(&task);
                    }
                    Some(TaskResult::Skipped {
                        reason: SkipReason::Manual,
                        message: SkipReason::Manual.label().into(),
                    })
                } else {
                    preflight.filter(|_| preparation_error.is_none())
                },
                None,
            )
        };
        let (outcome, record) = if let Some(result) = skip {
            let record = if let Some(info) = &info {
                SkippedRecord::recovered(&task.input, result.clone(), info.clone())
            } else {
                SkippedRecord::new(&task.input, result.clone())
            };
            let TaskResult::Skipped { reason, message } = result else {
                unreachable!("preflight and recovery return skips");
            };
            (
                TaskOutcome::Error(RunError::Skipped {
                    reason,
                    error: anyhow::anyhow!(message),
                }),
                Record::Skipped(record),
            )
        } else {
            if !predict {
                transcoder.set_cancellation(control.begin_task(offset + 1));
            }
            let request = TranscodeRequest {
                input: task.input.clone(),
                output: task.output.clone(),
                ..command.request.clone()
            };
            diagnostics.event(RunEvent::TaskStarted {
                index: offset + 1,
                total,
                input: task.input.clone(),
                output: (!predict).then(|| task.output.clone()),
            });
            diagnostics.begin_task();
            let outcome = execute_task(
                &command.operation,
                request,
                transcoder,
                diagnostics,
                recovery
                    .as_mut()
                    .map(|state| (state, task.relative.as_path())),
                preparation_error,
                control,
            )
            .await;
            if !predict {
                control.finish_task();
            }
            outcome
        };
        let result = outcome.result();
        if info.is_none()
            && !manual
            && let Some(state) = recovery.as_mut()
        {
            state.finish(&task.relative, &result);
        }
        report.write(&record, diagnostics);
        if let Some(info) = &info {
            batch.forced += usize::from(info.forced);
            diagnostics.recovered_task(&task.input, &result, info);
        }
        diagnostics.event(RunEvent::TaskFinished {
            index: offset + 1,
            total,
            input: task.input.clone(),
            result: result.clone(),
            recovery: info.clone(),
        });

        match outcome {
            TaskOutcome::Success => batch.succeeded += 1,
            TaskOutcome::Error(RunError::Skipped {
                reason: SkipReason::AlreadyCompleted,
                ..
            }) => batch.reused += 1,
            TaskOutcome::Error(error @ RunError::Skipped { reason, .. }) => {
                batch.skipped += 1;
                batch.resume_skipped += usize::from(reason == SkipReason::ResumeMismatch);
                batch.manual_skipped += usize::from(reason == SkipReason::Manual);
                if info.is_none() {
                    diagnostics.skipped_input(&task.input, &error);
                }
            }
            TaskOutcome::Error(error @ RunError::Failed { .. }) => {
                diagnostics.task_error(Some(&task.input), &error);
                let RunError::Failed { phase, error } = error else {
                    unreachable!()
                };
                batch.failures.push(TaskFailure {
                    input: task.input,
                    phase,
                    error,
                });
            }
            TaskOutcome::Error(RunError::Cancelled) => break,
        }

        if recovery.as_ref().is_some_and(Recovery::has_error) {
            break;
        }
    }

    report.finish(diagnostics);
    batch.state_error = recovery.as_mut().and_then(Recovery::take_error);
    if let Some(error) = &batch.state_error {
        diagnostics.error(error);
    }

    batch.cancelled = cancellation.is_cancelled();
    if total == batch.skipped
        && batch.resume_skipped == 0
        && batch.manual_skipped == 0
        && !batch.cancelled
    {
        diagnostics.error(&anyhow::anyhow!(
            "no video files found in {}",
            command.request.input.display()
        ));
    }
    diagnostics.batch_summary(&batch);

    RunOutcome::Batch(batch)
}

async fn execute_task(
    operation: &Operation,
    request: TranscodeRequest,
    transcoder: &Transcoder,
    diagnostics: &Diagnostics<'_>,
    recovery: Option<(&mut Recovery, &Path)>,
    preparation_error: Option<anyhow::Error>,
    control: &RunControl,
) -> (TaskOutcome, Record) {
    let predict = matches!(operation, Operation::Predict);
    let probe = if let Some(error) = preparation_error {
        diagnostics.event(RunEvent::PhaseChanged(RunPhase::PreparingOutput));
        Err(RunError::at(RunPhase::PreparingOutput, error))
    } else {
        diagnostics.event(RunEvent::PhaseChanged(RunPhase::Probing));
        recursive::probe_task(&request, transcoder, predict).await
    };
    match probe {
        Err(error @ RunError::Skipped { .. }) => {
            let record = Record::Skipped(SkippedRecord::new(&request.input, error.result()));
            (TaskOutcome::Error(error), record)
        }
        probe => match operation {
            Operation::Predict => {
                let mut record = PredictRecord::new(&request, transcoder.prediction_options());
                let outcome: TaskOutcome = match probe {
                    Ok(media) => {
                        transcoder
                            .predict_probed(request, media, diagnostics, &mut record)
                            .await
                    }
                    Err(error) => Err(error),
                }
                .into();
                let record = record.finish(outcome.result());
                (outcome, record)
            }
            Operation::Transcode => {
                let mut record = TranscodeRecord::new(&request);
                let outcome: TaskOutcome = match probe {
                    Ok(media) => {
                        transcoder
                            .run_probed(request, media, diagnostics, &mut record, recovery)
                            .await
                    }
                    Err(error) => Err(error),
                }
                .into();
                let outcome = match outcome {
                    TaskOutcome::Error(RunError::Cancelled) if !control.is_cancelled() => {
                        TaskOutcome::Error(RunError::Skipped {
                            reason: SkipReason::Manual,
                            error: anyhow::anyhow!(SkipReason::Manual.label()),
                        })
                    }
                    outcome => outcome,
                };
                let record = record.finish(outcome.result());
                (outcome, record)
            }
            Operation::Emulate(_) => unreachable!("recursive emulation is unsupported"),
        },
    }
}
