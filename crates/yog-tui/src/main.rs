mod app;
mod file_picker;
mod form;
mod ui;

use std::{io, process::ExitCode};

use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use ratatui::DefaultTerminal;

use app::App;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    #[cfg(feature = "cli")]
    {
        let args = std::env::args_os();
        if args.len() > 1 {
            return yog_cli::Args::parse_from(args).run().await;
        }
    }

    match run_tui().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

async fn run_tui() -> io::Result<()> {
    let mut terminal = ratatui::try_init()?;
    let result = run(&mut terminal).await;
    ratatui::restore();
    result
}

async fn run(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let mut app = App::default();
    let mut events = EventStream::new();

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        match events.next().await {
            Some(Ok(Event::Key(key)))
                if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
            {
                if app.handle_key(key) {
                    return Ok(());
                }
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(error),
            None => return Ok(()),
        }
    }
}
