mod app;
mod candidate;
mod encoding;
mod file_picker;
mod form;
mod run;
mod ui;

use std::{io, process::ExitCode, time::Duration};

use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use yog_runtime::{RunOutcome, event::RunEvent};

use app::{App, AppAction};
use form::RunRequest;

use crate::ui::DrawApp;

enum RuntimeMessage {
    Event(RunEvent),
    Finished(RunOutcome),
}

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
    let (runtime_tx, mut runtime_rx) = mpsc::unbounded_channel();
    let mut cancellation = None;
    let mut ticker = tokio::time::interval(Duration::from_millis(120));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        terminal.draw(|frame| frame.draw(&app))?;

        tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key)))
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    match app.handle_key(key) {
                        AppAction::None => {}
                        AppAction::Quit => return Ok(()),
                        AppAction::Start(request) => {
                            let token = CancellationToken::new();
                            spawn_run(*request, token.clone(), runtime_tx.clone());
                            cancellation = Some(token);
                        }
                        AppAction::Cancel => {
                            cancellation
                                .as_ref()
                                .expect("an active run must have a cancellation token")
                                .cancel();
                        }
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error),
                None => return Ok(()),
            },
            Some(message) = runtime_rx.recv() => match message {
                RuntimeMessage::Event(event) => app.handle_run_event(event),
                RuntimeMessage::Finished(outcome) => {
                    app.finish_run(outcome);
                    cancellation = None;
                }
            },
            _ = ticker.tick(), if app.is_animating() => app.tick(),
        }
    }
}

fn spawn_run(
    request: RunRequest,
    cancellation: CancellationToken,
    messages: mpsc::UnboundedSender<RuntimeMessage>,
) {
    tokio::spawn(async move {
        let events = messages.clone();
        let outcome = yog_runtime::run_with_events(
            request.command,
            request.options,
            request.config,
            cancellation,
            move |event| {
                let _ = events.send(RuntimeMessage::Event(event));
            },
        )
        .await;
        let _ = messages.send(RuntimeMessage::Finished(outcome));
    });
}
