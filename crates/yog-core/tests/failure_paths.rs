#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use yog_core::{error::Failure, ffprobe::Ffprobe};

static NEXT: AtomicU64 = AtomicU64::new(0);
// Avoid concurrently creating executables while another test forks: a child
// can briefly inherit an open writer until exec, causing ETXTBSY in a peer.
static TOOL_LIFETIME: Mutex<()> = Mutex::new(());
struct Tool {
    path: PathBuf,
    _guard: MutexGuard<'static, ()>,
}
impl Tool {
    fn new(body: &str) -> Self {
        let guard = TOOL_LIFETIME.lock().unwrap();
        let path = std::env::temp_dir().join(format!(
            "yog-fake-tool-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        use std::io::Write;
        write!(file, "#!/bin/sh\n{body}\n").unwrap();
        drop(file);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            path,
            _guard: guard,
        }
    }
    fn probe(&self) -> Ffprobe {
        Ffprobe::new(&self.path, Duration::from_millis(10))
    }
}
impl Drop for Tool {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[test]
fn failed_process_preserves_json_error_stderr_and_exit_status() {
    let tool = Tool::new(
        "printf '%s' '{\"error\":{\"code\":-22,\"string\":\"invalid input\"}}'; printf 'decoder detail' >&2; exit 9",
    );
    let error = tool
        .probe()
        .with_timeout(Duration::from_secs(5))
        .probe(Path::new("ignored"))
        .unwrap_err();
    assert!(
        matches!(error.reason, Failure::Probe(ref error) if error.code == -22 && error.string == "invalid input")
    );
    assert_eq!(error.status.unwrap().code(), Some(9));
    assert_eq!(error.stderr, b"decoder detail");
    assert!(error.secondary_io.is_empty());
}

#[test]
fn invalid_json_terminates_a_process_that_keeps_writing() {
    let tool = Tool::new(
        "printf 'before parse failure' >&2; printf 'invalid JSON'; while :; do printf 'more output'; done",
    );
    let started = Instant::now();
    let error = tool
        .probe()
        .with_timeout(Duration::from_secs(5))
        .probe(Path::new("ignored"))
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Json(_)));
    assert_eq!(error.stderr, b"before parse failure");
    assert!(error.status.is_some());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn cancellation_after_a_packet_keeps_diagnostics_and_reaps_child() {
    let tool = Tool::new(
        "printf 'before cancel' >&2; printf '%s' '{\"packets\":[{\"stream_index\":0},'; while :; do :; done",
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut count = 0;
    let error = tool
        .probe()
        .with_cancellation(Some(cancelled.clone()))
        .with_timeout(Duration::from_secs(5))
        .packets(Path::new("ignored"), None, "%+1", |_| {
            count += 1;
            cancelled.store(true, Ordering::Relaxed);
        })
        .unwrap_err();
    assert_eq!(count, 1);
    assert!(matches!(error.reason, Failure::Cancelled));
    assert_eq!(error.stderr, b"before cancel");
    assert!(error.status.is_some());
}

#[test]
fn timeout_keeps_diagnostics_and_pre_cancel_does_not_spawn() {
    let tool = Tool::new("printf 'before timeout' >&2; printf '{'; while :; do :; done");
    let error = tool
        .probe()
        .with_timeout(Duration::from_millis(100))
        .probe(Path::new("ignored"))
        .unwrap_err();
    assert!(matches!(error.reason, Failure::TimedOut));
    assert_eq!(error.stderr, b"before timeout");
    assert!(error.status.is_some());
    let error = Ffprobe::new("nonexistent-yog-tool", Duration::from_secs(1))
        .with_cancellation(Some(Arc::new(AtomicBool::new(true))))
        .probe(Path::new("ignored"))
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Cancelled));
    assert!(error.status.is_none());
}

#[test]
fn records_are_provisional_until_final_result_and_trailing_json_is_checked() {
    for trailer in ["}", "]} trailing"] {
        let tool = Tool::new(&format!(
            "printf '%s' '{{\"packets\":[{{\"stream_index\":0}}{trailer}'"
        ));
        let mut count = 0;
        let error = tool
            .probe()
            .with_timeout(Duration::from_secs(5))
            .packets(Path::new("ignored"), None, "%+1", |_| count += 1)
            .unwrap_err();
        assert_eq!(count, 1, "trailer={trailer}, error={error:?}");
        assert!(matches!(error.reason, Failure::Json(_)));
    }
}

#[test]
fn both_pipes_are_drained_and_successful_json_does_not_hide_nonzero_exit() {
    let tool = Tool::new(
        "printf '{\"streams\":[],\"padding\":\"'; i=0; while [ $i -lt 10000 ]; do printf 'stdout payload'; printf 'stderr payload' >&2; i=$((i+1)); done; printf '\"}'; exit 7",
    );
    let error = tool
        .probe()
        .with_timeout(Duration::from_secs(5))
        .probe(Path::new("ignored"))
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Exit));
    assert_eq!(error.status.unwrap().code(), Some(7));
    assert_eq!(error.stderr.len(), 140_000);
}
