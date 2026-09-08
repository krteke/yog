#![cfg(unix)]

use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;
use yog_core::{error::Failure, ffmpeg::Ffmpeg, ffprobe::Ffprobe};

static NEXT: AtomicU64 = AtomicU64::new(0);
// Avoid concurrently creating executables while another test forks: a child
// can briefly inherit an open writer until exec, causing ETXTBSY in a peer.
static TOOL_LIFETIME: Mutex<()> = Mutex::const_new(());
struct Tool {
    path: PathBuf,
    _guard: MutexGuard<'static, ()>,
}
impl Tool {
    async fn new(body: &str) -> Self {
        let guard = TOOL_LIFETIME.lock().await;
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
        Ffprobe::new(&self.path, Some(Duration::from_millis(10)))
    }
}
impl Drop for Tool {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[tokio::test]
async fn failed_process_preserves_json_error_stderr_and_exit_status() {
    let tool = Tool::new(
        "printf '%s' '{\"error\":{\"code\":-22,\"string\":\"invalid input\"}}'; printf 'decoder detail' >&2; exit 9",
    ).await;
    let error = tool
        .probe()
        .with_timeout(Some(Duration::from_secs(5)))
        .probe(Path::new("ignored"))
        .await
        .unwrap_err();
    assert!(
        matches!(error.reason, Failure::Probe(ref error) if error.code == -22 && error.string == "invalid input")
    );
    assert_eq!(error.status.unwrap().code(), Some(9));
    assert_eq!(error.stderr, b"decoder detail");
    assert!(error.secondary_io.is_empty());
}

#[tokio::test]
async fn invalid_json_terminates_a_process_that_keeps_writing() {
    let tool = Tool::new(
        "printf 'before parse failure' >&2; printf 'invalid JSON'; while :; do printf 'more output'; done",
    ).await;
    let started = Instant::now();
    let error = tool
        .probe()
        .with_timeout(Some(Duration::from_secs(5)))
        .probe(Path::new("ignored"))
        .await
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Json(_)));
    assert_eq!(error.stderr, b"before parse failure");
    assert!(error.status.is_some());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn cancellation_after_a_packet_keeps_diagnostics_and_reaps_child() {
    let tool = Tool::new(
        "printf 'before cancel' >&2; printf '%s' '{\"packets\":[{\"stream_index\":0},'; while :; do :; done",
    ).await;
    let cancelled = CancellationToken::new();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let error = tool
        .probe()
        .with_cancellation(cancelled.clone())
        .with_timeout(Some(Duration::from_secs(5)))
        .packets(Path::new("ignored"), None, "%+1", move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
            cancelled.cancel();
        })
        .await
        .unwrap_err();
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert!(matches!(error.reason, Failure::Cancelled));
    assert_eq!(error.stderr, b"before cancel");
    assert!(error.status.is_some());
}

#[tokio::test]
async fn timeout_keeps_diagnostics_and_pre_cancel_does_not_spawn() {
    let tool = Tool::new("printf 'before timeout' >&2; printf '{'; while :; do :; done").await;
    let error = tool
        .probe()
        .with_timeout(Some(Duration::from_millis(100)))
        .probe(Path::new("ignored"))
        .await
        .unwrap_err();
    assert!(matches!(error.reason, Failure::TimedOut));
    assert_eq!(error.stderr, b"before timeout");
    assert!(error.status.is_some());
    let error = Ffprobe::new("nonexistent-yog-tool", Some(Duration::from_secs(1)))
        .with_cancellation({
            let token = CancellationToken::new();
            token.cancel();
            token
        })
        .probe(Path::new("ignored"))
        .await
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Cancelled));
    assert!(error.status.is_none());
}

#[tokio::test]
async fn records_are_provisional_until_final_result_and_trailing_json_is_checked() {
    for trailer in ["}", "]} trailing"] {
        let tool = Tool::new(&format!(
            "printf '%s' '{{\"packets\":[{{\"stream_index\":0}}{trailer}'"
        ))
        .await;
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let error = tool
            .probe()
            .with_timeout(Some(Duration::from_secs(5)))
            .packets(Path::new("ignored"), None, "%+1", move |_| {
                seen.fetch_add(1, Ordering::Relaxed);
            })
            .await
            .unwrap_err();
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "trailer={trailer}, error={error:?}"
        );
        assert!(matches!(error.reason, Failure::Json(_)));
    }
}

#[tokio::test]
async fn both_pipes_are_drained_and_successful_json_does_not_hide_nonzero_exit() {
    let tool = Tool::new(
        "printf '{\"streams\":[],\"padding\":\"'; i=0; while [ $i -lt 10000 ]; do printf 'stdout payload'; printf 'stderr payload' >&2; i=$((i+1)); done; printf '\"}'; exit 7",
    ).await;
    let error = tool
        .probe()
        .with_timeout(Some(Duration::from_secs(5)))
        .probe(Path::new("ignored"))
        .await
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Exit));
    assert_eq!(error.status.unwrap().code(), Some(7));
    assert_eq!(error.stderr.len(), 140_000);
}

#[tokio::test]
async fn ffmpeg_drains_both_pipes_and_end_record_does_not_hide_failure() {
    let tool = Tool::new(
        r"i=0; while [ $i -lt 10000 ]; do printf 'frame=1\nprogress=continue\n'; printf 'stderr payload' >&2; i=$((i+1)); done; printf 'progress=end'; printf '\377' >&2; exit 7",
    ).await;
    let mut count = 0;
    let mut finished = false;
    let mut diagnostics = Vec::new();
    let error = Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)))
        .execute(
            std::iter::empty::<&str>(),
            |progress| {
                count += 1;
                finished = progress.finished;
            },
            |bytes| diagnostics.extend_from_slice(bytes),
        )
        .await
        .unwrap_err();
    assert_eq!(count, 10001);
    assert!(finished);
    assert!(matches!(error.reason, Failure::Exit));
    assert_eq!(error.status.unwrap().code(), Some(7));
    assert_eq!(diagnostics, error.stderr);
    assert_eq!(diagnostics.len(), 140001);
    assert_eq!(diagnostics.last(), Some(&255));
}

#[tokio::test]
async fn ffmpeg_callbacks_can_cancel_while_child_is_running() {
    for cancel_from_stderr in [false, true] {
        let tool = Tool::new(
            "printf 'diagnostic' >&2; printf 'frame=1\nprogress=continue\n'; while :; do :; done",
        )
        .await;
        let cancelled = CancellationToken::new();
        let error = Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)))
            .with_cancellation(cancelled.clone())
            .execute(
                std::iter::empty::<&str>(),
                |_| {
                    if !cancel_from_stderr {
                        cancelled.cancel();
                    }
                },
                |_| {
                    if cancel_from_stderr {
                        cancelled.cancel();
                    }
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error.reason, Failure::Cancelled));
        assert!(error.status.is_some());
        assert_eq!(error.stderr, b"diagnostic");
    }
}

#[tokio::test]
async fn ffmpeg_callback_panics_reap_before_propagation() {
    for panic_from_stderr in [false, true] {
        let tool =
            Tool::new("printf 'diagnostic' >&2; printf 'progress=continue\n'; while :; do :; done")
                .await;
        let started = Instant::now();
        let result = AssertUnwindSafe(async {
            Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)))
                .execute(
                    std::iter::empty::<&str>(),
                    |_| {
                        assert!(panic_from_stderr, "progress callback panic");
                    },
                    |_| {
                        assert!(!panic_from_stderr, "stderr callback panic");
                    },
                )
                .await
        })
        .catch_unwind()
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

#[tokio::test]
async fn ffmpeg_exit_without_progress_is_valid_but_silence_still_times_out() {
    let tool = Tool::new("exit 0").await;
    Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)))
        .execute(
            std::iter::empty::<&str>(),
            |_| panic!("unexpected progress"),
            |_| {},
        )
        .await
        .unwrap();
    drop(tool);
    let tool = Tool::new("printf 'waiting' >&2; while :; do :; done").await;
    let error = Ffmpeg::new(&tool.path, Some(Duration::from_millis(100)))
        .execute(std::iter::empty::<&str>(), |_| {}, |_| {})
        .await
        .unwrap_err();
    assert!(matches!(error.reason, Failure::TimedOut));
    assert!(error.status.is_some());
    assert_eq!(error.stderr, b"waiting");
}

#[tokio::test]
async fn explicit_hardware_decode_is_passed_before_input_and_failure_is_returned() {
    use yog_core::ffmpeg::{
        decoding::DecodingBackend,
        plan::{TranscodeRequest, VideoAction},
    };
    use yog_core::ffprobe::types::MediaInfo;
    let tool = Tool::new(
        r#"
[ "$1" = '-hide_banner' ] || exit 42
shift 5
[ "$1" = '-n' ] || exit 42
[ "$2" = '-hwaccel' ] && [ "$3" = 'vaapi' ] || exit 42
[ "$4" = '-hwaccel_device' ] && [ "$5" = '/nonexistent/yog device' ] || exit 42
[ "$6" = '-i' ] && [ "$7" = 'input.mkv' ] || exit 42
printf 'device initialization failed' >&2
exit 17
"#,
    )
    .await;
    let media: MediaInfo =
        serde_json::from_str(r#"{"streams":[{"index":0,"codec_type":"video"}]}"#).unwrap();
    let plan = TranscodeRequest::mkv("input.mkv", "output.mkv")
        .with_decoding(DecodingBackend::Vaapi(Some(
            "/nonexistent/yog device".into(),
        )))
        .with_video(VideoAction::encode_x264(None, None))
        .plan(&media);
    let error = Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)))
        .execute(plan.args(), |_| {}, |_| {})
        .await
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Exit));
    assert_eq!(error.status.unwrap().code(), Some(17));
    assert_eq!(error.stderr, b"device initialization failed");
}

#[tokio::test]
async fn continuous_progress_does_not_extend_the_process_deadline() {
    let tool = Tool::new("while :; do printf 'frame=1\nprogress=continue\n'; done").await;
    let started = Instant::now();
    let mut records = 0;
    let error = Ffmpeg::new(&tool.path, Some(Duration::from_millis(100)))
        .execute(std::iter::empty::<&str>(), |_| records += 1, |_| {})
        .await
        .unwrap_err();
    assert!(records > 0);
    assert!(matches!(error.reason, Failure::TimedOut));
    assert!(error.status.is_some());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn another_async_task_can_cancel_a_probe_blocked_in_json_parsing() {
    let tool = Tool::new("printf '{'; while :; do :; done").await;
    let cancelled = CancellationToken::new();
    let probe = Ffprobe::new(&tool.path, None).with_cancellation(cancelled.clone());
    let task = tokio::spawn(async move { probe.probe(Path::new("ignored")).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancelled.cancel();
    let error = task.await.unwrap().unwrap_err();
    assert!(matches!(error.reason, Failure::Cancelled));
    assert!(error.status.is_some());
}

#[tokio::test]
async fn blocking_parser_callback_panics_are_reaped_before_propagation() {
    let tool =
        Tool::new("printf '{\"packets\":[{\"stream_index\":%s},' \"$$\"; while :; do :; done")
            .await;
    let pid = Arc::new(AtomicUsize::new(0));
    let observed = pid.clone();
    let probe = Ffprobe::new(&tool.path, Some(Duration::from_secs(5)));
    let result =
        AssertUnwindSafe(
            probe.packets(Path::new("ignored"), None, "%+1", move |packet| {
                observed.store(packet.stream_index, Ordering::Relaxed);
                panic!("packet callback panic");
            }),
        )
        .catch_unwind()
        .await;
    assert!(result.is_err());
    assert_ne!(pid.load(Ordering::Relaxed), 0);
    #[cfg(target_os = "linux")]
    assert!(!Path::new(&format!("/proc/{}", pid.load(Ordering::Relaxed))).exists());
}
