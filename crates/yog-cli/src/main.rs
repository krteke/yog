mod args;
mod config;
mod decoding;
mod diagnostics;
mod error;
mod output;
mod progress;
mod recursive;
mod transcode;
mod validate;
mod verify;

use anyhow::Context;
use args::Args;
use clap::Parser;
use std::process::ExitCode;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

use crate::{error::RunError, validate::Validate};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = match Args::parse().validate() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error:#}");
            return ExitCode::FAILURE;
        }
    };

    let recursive = args.recursive;
    let predict = args.predict;
    if let Err(error) = config::init(args.config.as_deref()) {
        eprintln!("{error:#}");
        return ExitCode::FAILURE;
    }

    let (request, options) = args.into_request().unwrap_or_else(|error| error.exit());

    let cancelled = CancellationToken::new();
    let diagnostics = diagnostics::Diagnostics::new(options.verbose);
    let transcoder = transcode::Transcoder::new(&options, cancelled.clone());
    let mut signal_task = None;
    let result = match signal(SignalKind::interrupt()).context("cannot register Ctrl+C handler") {
        Ok(mut interrupts) => {
            let token = cancelled.clone();
            signal_task = Some(tokio::spawn(async move {
                interrupts.recv().await;
                token.cancel();
            }));
            async {
                if recursive {
                    let tasks = recursive::discover(request, &transcoder).await?;
                    let total = tasks.len();
                    let mut succeeded = 0;
                    let mut failed = 0;
                    for (request, media) in tasks {
                        let input = request.input.clone();
                        diagnostics.begin_task();
                        let result = if predict {
                            transcoder
                                .predict_probed(request, media, &diagnostics)
                                .await
                        } else {
                            transcoder.run_probed(request, media, &diagnostics).await
                        };
                        match result {
                            Ok(()) => succeeded += 1,
                            Err(RunError::Failed(error)) => {
                                failed += 1;
                                diagnostics.task_error(&input, &error);
                            }
                            Err(RunError::Cancelled) => break,
                        }
                    }

                    let cancelled = cancelled.is_cancelled();
                    diagnostics.batch_summary(total, succeeded, failed, cancelled);
                    return Ok(if cancelled {
                        ExitCode::from(130)
                    } else if failed > 0 {
                        ExitCode::FAILURE
                    } else {
                        ExitCode::SUCCESS
                    });
                } else if predict {
                    transcoder.predict(request, &diagnostics).await?;
                } else {
                    transcoder.run(request, &diagnostics).await?;
                }
                Ok(ExitCode::SUCCESS)
            }
            .await
        }
        Err(error) => Err(RunError::from(error)),
    };

    let exit = match result {
        Ok(exit) => exit,
        Err(RunError::Cancelled) => {
            println!("cancelled");
            ExitCode::from(130)
        }
        Err(RunError::Failed(error)) => {
            diagnostics.error(&error);
            ExitCode::FAILURE
        }
    };

    if let Some(task) = signal_task {
        task.abort();
        let _ = task.await;
    }

    exit
}
