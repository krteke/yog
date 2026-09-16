use clap::Parser;
use std::process::ExitCode;
use yog_cli::Args;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = Args::parse();
    args.run().await
}
