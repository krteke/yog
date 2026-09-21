use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    #[cfg(feature = "cli")]
    {
        use yog_cli::Args;

        let args = std::env::args_os();

        let exit = if args.len() == 1 {
            ExitCode::SUCCESS
        } else {
            let args = Args::parse_from(args);
            args.run().await
        };
        return exit;
    }

    #[cfg(not(feature = "cli"))]
    todo!()
}
