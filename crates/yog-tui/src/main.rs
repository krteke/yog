use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    #[cfg(feature = "cli")]
    {
        use yog_cli::Args;

        let args = std::env::args_os();

        if args.len() == 1 {
            todo!()
        } else {
            let args = Args::parse_from(args);
            return args.run().await;
        }
    }
}
