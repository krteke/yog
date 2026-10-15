use crate::{
    Command, Config, Options, RunOutcome,
    control::RunControl,
    event::RunEvent,
    recovery::{RecoveryAction, RecoveryDecision, RecoveryRequest, RecoveryScope},
};
use anyhow::{Context, Result, bail};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::{
    io::{self, IsTerminal, Write},
    os::fd::AsFd,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub async fn run(
    command: Command,
    options: Options,
    config: Config,
    control: RunControl,
) -> RunOutcome {
    let cancellation = control.cancellation();
    let terminal_output = options.terminal_output;
    let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let run = crate::run_with_events(command, options, config, control, move |event| {
        if let RunEvent::RecoveryDecisionRequired(request) = event {
            let _ = requests.send(request);
        }
    });
    tokio::pin!(run);
    loop {
        tokio::select! {
            outcome = &mut run => return outcome,
            Some(request) = receiver.recv() => {
                match prompt(&request, terminal_output, &cancellation).await {
                    Ok(Some(decision)) => { request.respond(decision.action, decision.apply_to_similar); }
                    Ok(None) => {
                        drop(request);
                        return run.await;
                    }
                    Err(error) => {
                        request.fail(format!("{error:#}"));
                        return run.await;
                    }
                }
            }
        }
    }
}

async fn prompt(
    request: &RecoveryRequest,
    terminal_output: bool,
    cancellation: &CancellationToken,
) -> Result<Option<RecoveryDecision>> {
    if !terminal_output || !io::stdin().is_terminal() {
        let option = match request.scope {
            RecoveryScope::Batch => "--resume-batch-mismatch",
            RecoveryScope::File => "--resume-file-mismatch",
        };
        bail!(
            "{} for {} requires an interactive terminal; choose {option} abort|force|skip|restart for unattended execution",
            request.title(),
            request.input.display()
        );
    }
    let mut stderr = io::stderr();
    writeln!(stderr, "\n{}: {}", request.title(), request.input.display())?;
    writeln!(stderr, "State: {}", request.state_path.display())?;
    if let Some(output) = &request.output {
        writeln!(stderr, "Recorded output: {}", output.display())?;
    }
    for difference in &request.differences {
        writeln!(
            stderr,
            "{}\n  Recorded: {}\n  Current:  {}",
            difference.kind, difference.recorded, difference.current
        )?;
    }
    for (index, action) in RecoveryAction::ALL.into_iter().enumerate() {
        writeln!(
            stderr,
            "{}. {} — {}",
            index + 1,
            request.action_label(action),
            request.action_detail(action)
        )?;
    }
    writeln!(
        stderr,
        "Enter 1–4; append 'all' to apply to later differences of the same kind in this run. Ctrl+C cancels."
    )?;
    write!(stderr, "> ")?;
    stderr.flush()?;
    let stdin = io::stdin();
    let mut line = Vec::new();
    loop {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let mut fds = [PollFd::from_borrowed_fd(stdin.as_fd(), PollFlags::IN)];
        // Read only ready terminal bytes: an unfinished line must not block Ctrl+C.
        if poll(&mut fds, Some(&Timespec::default())).context("cannot poll terminal input")? > 0 {
            let mut byte = [0];
            if rustix::io::read(&stdin, &mut byte).context("cannot read recovery choice")? == 0 {
                bail!("terminal input ended while waiting for a recovery decision");
            }
            if byte[0] == b'\n' {
                if let Some(decision) = parse_response(&String::from_utf8_lossy(&line)) {
                    return Ok(Some(decision));
                }
                line.clear();
                write!(stderr, "Choose 1–4, optionally followed by 'all': ")?;
                stderr.flush()?;
            } else {
                line.push(byte[0]);
            }
        } else {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(None),
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    }
}

fn parse_response(line: &str) -> Option<RecoveryDecision> {
    let mut words = line.split_whitespace();
    let action = match words.next()? {
        "1" | "abort" => RecoveryAction::Abort,
        "2" | "force" => RecoveryAction::Force,
        "3" | "skip" => RecoveryAction::Skip,
        "4" | "restart" => RecoveryAction::Restart,
        _ => return None,
    };
    let apply_to_similar = match words.next() {
        None => false,
        Some("all") => true,
        _ => return None,
    };
    if words.next().is_some() {
        return None;
    }
    Some(RecoveryDecision {
        action,
        apply_to_similar,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_all_explicitly_selects_record_reuse_for_similar_differences() {
        let decision = parse_response("2 all").unwrap();
        assert_eq!(decision.action, RecoveryAction::Force);
        assert!(decision.apply_to_similar);
    }

    #[test]
    fn an_empty_terminal_response_does_not_make_a_decision() {
        assert!(parse_response("").is_none());
    }

    #[test]
    fn unknown_response_scope_is_not_treated_as_apply_to_all() {
        assert!(parse_response("force everything").is_none());
    }
}
