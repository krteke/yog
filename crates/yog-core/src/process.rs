use crate::error::{Error, Failure};
use std::ffi::OsStr;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{ChildStdout, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct Output<T> {
    pub program: std::path::PathBuf,
    pub status: ExitStatus,
    pub value: T,
    pub stderr: Vec<u8>,
}

impl<T> Output<T> {
    pub fn failure(self, reason: Failure) -> Error {
        Error {
            program: self.program,
            reason,
            status: Some(self.status),
            stderr: self.stderr,
            secondary_io: Vec::new(),
        }
    }
}

/// Parse stdout while the process runs. Stderr is drained independently without
/// a hidden retention limit. A decoder failure stops and reaps the child.
pub(crate) fn run<T, I, O, D, S>(
    program: &Path,
    args: I,
    timeout: Duration,
    cancellation: Option<Arc<AtomicBool>>,
    decode: D,
    mut on_stderr: S,
) -> Result<Output<T>, Error>
where
    T: Send,
    I: IntoIterator<Item = O>,
    O: AsRef<OsStr>,
    D: FnOnce(BufReader<ChildStdout>) -> Result<T, Failure> + Send,
    S: FnMut(&[u8]) + Send,
{
    let mut failure = Error {
        program: program.to_owned(),
        reason: Failure::Cancelled,
        status: None,
        stderr: Vec::new(),
        secondary_io: Vec::new(),
    };
    if cancellation
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
    {
        return Err(failure);
    }
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error {
            program: program.to_owned(),
            reason: Failure::Io(error),
            status: None,
            stderr: Vec::new(),
            secondary_io: Vec::new(),
        })?;
    let stdout = child.stdout.take().expect("stdout configured as piped");
    let mut stderr = child.stderr.take().expect("stderr configured as piped");
    thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        let reader = scope.spawn(move || {
            let result = decode(BufReader::new(stdout));
            // The receiver lives until this reader has been joined.
            tx.send(result).ok();
        });
        let (stderr_tx, stderr_rx) = mpsc::channel();
        let diagnostics = scope.spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            let result = loop {
                match stderr.read(&mut buffer) {
                    Ok(0) => break Ok(()),
                    Ok(length) => {
                        bytes.extend_from_slice(&buffer[..length]);
                        on_stderr(&buffer[..length]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => break Err(error),
                }
            };
            stderr_tx.send(()).ok();
            (bytes, result)
        });
        let started = Instant::now();
        let mut value = None;
        let mut reason = None;
        let mut stderr_done = false;
        loop {
            if !stderr_done {
                match stderr_rx.try_recv() {
                    Ok(()) => stderr_done = true,
                    Err(mpsc::TryRecvError::Empty) => {}
                    // Reap the child before propagating a callback panic.
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            }
            if failure.status.is_none() {
                match child.try_wait() {
                    Ok(status) => failure.status = status,
                    Err(error) => {
                        reason = Some(Failure::Io(error));
                        break;
                    }
                }
            }
            if value.is_none() {
                match rx.try_recv() {
                    Ok(Ok(parsed)) => value = Some(parsed),
                    Ok(Err(error)) => {
                        reason = Some(error);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        // A panicking decoder is a programmer failure. Reap before propagating it.
                        break;
                    }
                }
            }
            if failure.status.is_some() && value.is_some() && stderr_done {
                break;
            }
            if cancellation
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
            {
                reason = Some(Failure::Cancelled);
                break;
            }
            if started.elapsed() >= timeout {
                reason = Some(Failure::TimedOut);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if failure.status.is_none() {
            if let Err(error) = child.kill() {
                failure.secondary_io.push(error);
            }
            match child.wait() {
                Ok(status) => failure.status = Some(status),
                Err(error) => failure.secondary_io.push(error),
            }
        }
        let joined = reader.join();
        let (stderr, read_result) = match diagnostics.join() {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        failure.stderr = stderr;
        if let Err(panic) = joined {
            std::panic::resume_unwind(panic);
        }
        if let Err(error) = read_result {
            if reason.is_some() {
                failure.secondary_io.push(error);
            } else {
                reason = Some(Failure::Io(error));
            }
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
    })
}
