use crate::error::{Error, Failure};
use crate::process::Output;

use futures_util::FutureExt;
use std::{
    ffi::{OsStr, OsString},
    fmt,
    future::{Future, pending},
    panic::{AssertUnwindSafe, resume_unwind},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::{ChildStdout, Command as TokioCommand},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct Program {
    pub path: PathBuf,
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
}

impl Program {
    pub fn build<I, O>(&self, args: I) -> Command<'_>
    where
        I: IntoIterator<Item = O>,
        O: AsRef<OsStr>,
    {
        Command {
            program: self,
            args: args
                .into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect(),
        }
    }
}

pub struct Command<'a> {
    program: &'a Program,
    args: Vec<OsString>,
}

impl fmt::Display for Command<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.program.path.to_string_lossy())?;
        for arg in &self.args {
            write!(formatter, " {}", arg.to_string_lossy())?;
        }
        Ok(())
    }
}

impl Command<'_> {
    pub async fn run<T, D, F, S>(self, decode: D, mut on_stderr: S) -> Result<Output<T>, Error>
    where
        D: FnOnce(ChildStdout) -> F,
        F: Future<Output = Result<T, Failure>>,
        S: FnMut(&[u8]),
    {
        let mut failure = Error {
            program: self.program.path.clone(),
            reason: Failure::Cancelled,
            status: None,
            stderr: Vec::new(),
            secondary_io: Vec::new(),
        };
        if self.program.cancellation.is_cancelled() {
            return Err(failure);
        }

        log::info!(target: "yog::command", "executing command: {self}");
        let mut child = TokioCommand::new(&self.program.path)
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| Error {
                program: self.program.path.clone(),
                reason: Failure::Io(error),
                status: None,
                stderr: Vec::new(),
                secondary_io: Vec::new(),
            })?;
        let stdout = child.stdout.take().expect("stdout configured as piped");
        let mut stderr = child.stderr.take().expect("stderr configured as piped");
        let timer = self.program.timeout.map(tokio::time::sleep);
        let deadline = async {
            match timer {
                Some(timer) => timer.await,
                None => pending().await,
            }
        };
        tokio::pin!(deadline);
        let mut value = None;
        let mut reason = None;
        let mut panic = None;
        {
            let decoded = AssertUnwindSafe(async { decode(stdout).await }).catch_unwind();
            let diagnostics = AssertUnwindSafe(async {
                let mut buffer = [0; 8192];
                loop {
                    let length = stderr.read(&mut buffer).await?;
                    if length == 0 {
                        return Ok::<_, std::io::Error>(());
                    }
                    failure.stderr.extend_from_slice(&buffer[..length]);
                    on_stderr(&buffer[..length]);
                }
            })
            .catch_unwind();

            tokio::pin!(decoded, diagnostics);
            let mut stdout_done = false;
            let mut stderr_done = false;

            while failure.status.is_none() || !stdout_done || !stderr_done {
                tokio::select! {
                    _ = self.program.cancellation.cancelled() => {
                        reason = Some(Failure::Cancelled);
                        break;
                    }
                    _ = &mut deadline => {
                        reason = Some(Failure::TimedOut);
                        break;
                    }
                    result = child.wait(), if failure.status.is_none() => {
                        match result {
                            Ok(status) => failure.status = Some(status),
                            Err(error) => { reason = Some(Failure::Io(error)); break; }
                        }
                    }
                    result = &mut decoded, if !stdout_done => {
                        stdout_done = true;
                        match result {
                            Ok(Ok(parsed)) => value = Some(parsed),
                            Ok(Err(error)) => { reason = Some(error); break; }
                            Err(payload) => { panic = Some(payload); break; }
                        }
                    }
                    result = &mut diagnostics, if !stderr_done => {
                        stderr_done = true;
                        match result {
                            Ok(Ok(())) => {},
                            Ok(Err(error)) => { reason = Some(Failure::Io(error)); break; }
                            Err(payload) => { panic = Some(payload); break; }
                        }
                    }
                }
            }
            if failure.status.is_none() {
                if let Err(error) = child.start_kill() {
                    failure.secondary_io.push(error);
                }
                match child.wait().await {
                    Ok(status) => failure.status = Some(status),
                    Err(error) => failure.secondary_io.push(error),
                }
            }
            if !stdout_done && let Err(payload) = decoded.await {
                panic = Some(payload);
            }
            if !stderr_done {
                match diagnostics.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => failure.secondary_io.push(error),
                    Err(payload) => panic = Some(payload),
                }
            }
        }
        if let Some(payload) = panic {
            resume_unwind(payload);
        }
        if let Some(reason) = reason {
            failure.reason = reason;
            return Err(failure);
        }
        Ok(Output {
            program: failure.program,
            status: failure.status.expect("child reaped"),
            value: value.expect("decoder completed"),
            stderr: failure.stderr,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_display_matches_the_debug_log_format() {
        let program = Program {
            path: "ffprobe".into(),
            timeout: None,
            cancellation: CancellationToken::new(),
        };
        let command = program.build([
            "-v",
            "error",
            "-of",
            "json",
            "-show_error",
            "-show_format",
            "-show_streams",
            "-show_chapters",
            "-show_programs",
            "-show_pixel_formats",
            "--",
            "/tmp/sample-1",
        ]);

        assert_eq!(
            command.to_string(),
            "ffprobe -v error -of json -show_error -show_format -show_streams -show_chapters -show_programs -show_pixel_formats -- /tmp/sample-1"
        );
    }
}
