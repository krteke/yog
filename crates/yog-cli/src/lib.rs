mod args;
mod decoding;

use std::{ffi::OsString, process::ExitCode};

use clap::Parser;

pub use args::Args;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use yog_runtime::{Config, RunStatus};

impl Args {
    pub fn parse_from<I, T>(iter: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        <Self as Parser>::parse_from(iter)
    }

    pub async fn run(self) -> ExitCode {
        let (config_path, command, options) =
            self.into_runtime().unwrap_or_else(|error| error.exit());
        init_logger(options.verbose, options.terminal_output);

        let config = match Config::load(config_path.as_deref()) {
            Ok(config) => config,
            Err(error) => {
                if options.terminal_output {
                    eprintln!("{error:#}");
                }
                return ExitCode::FAILURE;
            }
        };

        let cancelled = CancellationToken::new();
        let mut interrupts = match signal(SignalKind::interrupt()) {
            Ok(interrupts) => interrupts,
            Err(error) => {
                if options.terminal_output {
                    eprintln!("cannot register Ctrl+C handler: {error}");
                }
                return ExitCode::FAILURE;
            }
        };
        let token = cancelled.clone();
        let signal_task = tokio::spawn(async move {
            interrupts.recv().await;
            token.cancel();
        });
        let outcome = yog_runtime::run(command, options, config, cancelled).await;
        signal_task.abort();
        let _ = signal_task.await;

        match outcome.status() {
            RunStatus::Success => ExitCode::SUCCESS,
            RunStatus::Failure => ExitCode::FAILURE,
            RunStatus::Cancelled => ExitCode::from(130),
        }
    }
}

pub fn init_logger(verbose: bool, terminal_output: bool) {
    let mut logger = if terminal_output {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(if verbose {
            "debug"
        } else {
            "warn"
        }))
    } else {
        let mut logger = env_logger::Builder::new();
        logger.parse_filters("off");
        logger
    };
    logger.format_timestamp(None).format_target(false).init();
}
