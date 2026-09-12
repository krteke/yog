mod args;
mod config;
mod decoding;
mod diagnostics;
mod error;
mod output;
mod progress;
mod recursive;
mod terminal;
mod transcode;
mod verify;

use anyhow::Context;
use args::Args;
use clap::Parser;
use std::process::ExitCode;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

use crate::error::RunError;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = Args::parse();
    let recursive = args.recursive;
    if let Err(error) = config::init(args.config.as_deref()) {
        eprintln!("{error:#}");
        return ExitCode::FAILURE;
    }

    let (request, options) = args.into_request().unwrap_or_else(|error| error.exit());

    let _terminal = match terminal::NonblockingStderr::new() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("{error:#}");
            return ExitCode::FAILURE;
        }
    };

    let cancelled = CancellationToken::new();
    let diagnostics = diagnostics::Diagnostics::new(options.verbose, cancelled.clone());
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
                    let tasks = recursive::discover(request, &transcoder, &diagnostics).await?;
                    for (request, media) in tasks {
                        transcoder.run_probed(request, media, &diagnostics).await?;
                    }
                } else {
                    transcoder.run(request, &diagnostics).await?;
                }
                Ok(())
            }
            .await
        }
        Err(error) => Err(RunError::from(error)),
    };

    let exit = match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(RunError::Cancelled) => {
            diagnostics.write("cancelled\n".as_bytes());
            ExitCode::from(130)
        }
        Err(RunError::Failed(error)) => {
            diagnostics.error(&error);
            ExitCode::FAILURE
        }
    };

    diagnostics.finish().await;
    if let Some(task) = signal_task {
        task.abort();
        let _ = task.await;
    }

    exit
}
