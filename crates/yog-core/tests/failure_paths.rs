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
use yog_core::{
    error::Failure,
    ffmpeg::{
        Ffmpeg,
        plan::{TranscodePlan, TranscodeRequest},
    },
    ffprobe::Ffprobe,
};

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
    async fn plan(&self) -> TranscodePlan {
        TranscodeRequest::mkv("input", "unused-output")
            .plan(
                &serde_json::from_str(r#"{"streams":[{"index":0,"codec_type":"audio"}]}"#).unwrap(),
                &Ffmpeg::new(&self.path, None),
            )
            .await
            .unwrap()
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

async fn assert_invalid_packet_trailer(trailer: &str) {
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

async fn assert_ffmpeg_callback_cancellation(cancel_from_stderr: bool) {
    let tool = Tool::new(
        "printf 'diagnostic' >&2; printf 'frame=1\nprogress=continue\n'; while :; do :; done",
    )
    .await;
    let cancelled = CancellationToken::new();
    let ffmpeg =
        Ffmpeg::new(&tool.path, Some(Duration::from_secs(5))).with_cancellation(cancelled.clone());
    let plan = tool.plan().await;
    let error = ffmpeg
        .build(&plan)
        .unwrap()
        .run(
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

async fn assert_ffmpeg_callback_panic(panic_from_stderr: bool) {
    let tool =
        Tool::new("printf 'diagnostic' >&2; printf 'progress=continue\n'; while :; do :; done")
            .await;
    let started = Instant::now();
    let ffmpeg = Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)));
    let plan = tool.plan().await;
    let result = AssertUnwindSafe(async {
        ffmpeg
            .build(&plan)
            .unwrap()
            .run(
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

#[tokio::test]
async fn packet_fingerprints_separate_stream_order_payload_and_presentation_time() {
    let tool = Tool::new(r#"
for last do :; done
case "$last" in
  original) printf '%s' '{"packets":[{"stream_index":4,"data_hash":"A","pts_time":"0.000000"},{"stream_index":9,"data_hash":"C"},{"stream_index":4,"data_hash":"B","pts_time":"0.020000"},{"stream_index":20,"data_hash":"ignored"}]}' ;;
  interleaved) printf '%s' '{"packets":[{"stream_index":9,"data_hash":"C"},{"stream_index":4,"data_hash":"A","pts_time":"0.000000"},{"stream_index":20,"data_hash":"different"},{"stream_index":4,"data_hash":"B","pts_time":"0.020000"}]}' ;;
  payload) printf '%s' '{"packets":[{"stream_index":4,"data_hash":"B","pts_time":"0.000000"},{"stream_index":9,"data_hash":"C"},{"stream_index":4,"data_hash":"A","pts_time":"0.020000"}]}' ;;
  timing) printf '%s' '{"packets":[{"stream_index":4,"data_hash":"A","pts_time":"0.010000"},{"stream_index":9,"data_hash":"C"},{"stream_index":4,"data_hash":"B","pts_time":"0.030000"}]}' ;;
  missing) printf '%s' '{"packets":[{"stream_index":4}]}' ;;
esac
"#).await;
    let probe = tool.probe().with_timeout(Some(Duration::from_secs(5)));
    let original = probe
        .packet_fingerprints(Path::new("original"), &[4, 9])
        .await
        .unwrap()
        .output;
    let interleaved = probe
        .packet_fingerprints(Path::new("interleaved"), &[4, 9])
        .await
        .unwrap()
        .output;
    assert_eq!(original, interleaved);
    assert_eq!(original.len(), 2);
    let payload = probe
        .packet_fingerprints(Path::new("payload"), &[4, 9])
        .await
        .unwrap()
        .output;
    assert_eq!(original[&4].packets, payload[&4].packets);
    assert_ne!(original[&4].sha256, payload[&4].sha256);
    assert_eq!(original[&4].timing_sha256, payload[&4].timing_sha256);
    assert_eq!(original[&9], payload[&9]);
    let timing = probe
        .packet_fingerprints(Path::new("timing"), &[4, 9])
        .await
        .unwrap()
        .output;
    assert_eq!(original[&4].sha256, timing[&4].sha256);
    assert_ne!(original[&4].timing_sha256, timing[&4].timing_sha256);
    let error = probe
        .packet_fingerprints(Path::new("missing"), &[4])
        .await
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Json(_)));
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
async fn packet_records_remain_provisional_when_the_array_is_unfinished() {
    assert_invalid_packet_trailer("}").await;
}

#[tokio::test]
async fn packet_records_remain_provisional_when_json_has_trailing_data() {
    assert_invalid_packet_trailer("]} trailing").await;
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
    let ffmpeg = Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)));
    let plan = tool.plan().await;
    let error = ffmpeg
        .build(&plan)
        .unwrap()
        .run(
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
async fn ffmpeg_progress_callback_can_cancel_while_child_is_running() {
    assert_ffmpeg_callback_cancellation(false).await;
}

#[tokio::test]
async fn ffmpeg_stderr_callback_can_cancel_while_child_is_running() {
    assert_ffmpeg_callback_cancellation(true).await;
}

#[tokio::test]
async fn ffmpeg_progress_callback_panic_reaps_before_propagation() {
    assert_ffmpeg_callback_panic(false).await;
}

#[tokio::test]
async fn ffmpeg_stderr_callback_panic_reaps_before_propagation() {
    assert_ffmpeg_callback_panic(true).await;
}

#[tokio::test]
async fn ffmpeg_exit_without_progress_is_valid_but_silence_still_times_out() {
    let tool = Tool::new("exit 0").await;
    let ffmpeg = Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)));
    let plan = tool.plan().await;
    ffmpeg
        .build(&plan)
        .unwrap()
        .run(|_| panic!("unexpected progress"), |_| {})
        .await
        .unwrap();
    drop(tool);
    let tool = Tool::new("printf 'waiting' >&2; while :; do :; done").await;
    let ffmpeg = Ffmpeg::new(&tool.path, Some(Duration::from_millis(100)));
    let plan = tool.plan().await;
    let error = ffmpeg
        .build(&plan)
        .unwrap()
        .run(|_| {}, |_| {})
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
if [ "$2" = '-h' ]; then
    printf 'Encoder libx264 [H264]:\n Supported pixel formats: yuv420p\n'
    exit 0
fi
shift 5
[ "$1" = '-n' ] || exit 42
[ "$2" = '-hwaccel:0' ] && [ "$3" = 'vaapi' ] || exit 42
[ "$4" = '-hwaccel_device:0' ] && [ "$5" = '/nonexistent/yog device' ] || exit 42
[ "$6" = '-hwaccel_output_format:0' ] && [ "$7" = 'vaapi' ] || exit 42
[ "$8" = '-i' ] && [ "$9" = 'input.mkv' ] || exit 42
printf 'device initialization failed' >&2
exit 17
"#,
    )
    .await;
    let mut media: MediaInfo =
        serde_json::from_str(include_str!("../src/ffmpeg/test_pixel_formats.json")).unwrap();
    media.streams =
        serde_json::from_str(r#"[{"index":0,"codec_type":"video","pix_fmt":"yuv420p"}]"#).unwrap();
    let ffmpeg = Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)));
    let plan = TranscodeRequest::mkv("input.mkv", "output.mkv")
        .with_decoding(DecodingBackend::Vaapi(Some(
            "/nonexistent/yog device".into(),
        )))
        .with_video(VideoAction::encode_x264(None, None))
        .plan(&media, &ffmpeg)
        .await
        .unwrap();
    let error = ffmpeg
        .build(&plan)
        .unwrap()
        .run(|_| {}, |_| {})
        .await
        .unwrap_err();
    assert!(matches!(error.reason, Failure::Exit));
    assert_eq!(error.status.unwrap().code(), Some(17));
    assert_eq!(error.stderr, b"device initialization failed");
}

#[tokio::test]
async fn native_attachment_is_dumped_and_reattached_by_the_transcode_process() {
    use yog_core::ffmpeg::plan::VideoAction;
    use yog_core::ffprobe::types::MediaInfo;

    let tool = Tool::new(
        r#"
if [ "$2" = '-h' ]; then
    printf 'Encoder libx264 [H264]:\n Supported pixel formats: yuv420p\n'
    exit 0
fi
seen_input=0
seen_delta=0
attachment=''
last=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        -dump_attachment:3)
            [ "$seen_input" = 0 ] || exit 41
            attachment=$2
            printf original-font > "$attachment"
            shift 2
            ;;
        -i)
            seen_input=1
            shift 2
            ;;
        -map)
            [ "$2" != '0:3' ] || exit 42
            shift 2
            ;;
        -attach)
            [ "$seen_input" = 1 ] || exit 43
            [ "$2" = "$attachment" ] || exit 44
            [ "$(cat "$2")" = original-font ] || exit 45
            shift 2
            ;;
        -max_interleave_delta)
            [ "$2" = 0 ] || exit 46
            seen_delta=1
            shift 2
            ;;
        *)
            last=$1
            shift
            ;;
    esac
done
[ -n "$attachment" ] || exit 47
[ "$seen_delta" = 1 ] || exit 48
printf encoded > "$last"
"#,
    )
    .await;
    let mut media: MediaInfo =
        serde_json::from_str(include_str!("../src/ffmpeg/test_pixel_formats.json")).unwrap();
    media.streams = serde_json::from_str(
        r#"[
            {"index":0,"codec_type":"video","pix_fmt":"yuv420p"},
            {"index":1,"codec_type":"audio"},
            {"index":3,"codec_type":"attachment","tags":{"filename":"font.ttf","mimetype":"font/ttf"}}
        ]"#,
    )
    .unwrap();
    let ffmpeg = Ffmpeg::new(&tool.path, Some(Duration::from_secs(5)));
    let output = tool.path.with_extension("output.mkv");
    let plan = TranscodeRequest::mkv("input.mkv", &output)
        .with_video(VideoAction::encode_x264(None, None))
        .plan(&media, &ffmpeg)
        .await
        .unwrap();
    ffmpeg
        .build(&plan)
        .unwrap()
        .run(|_| {}, |_| {})
        .await
        .unwrap();
    assert_eq!(fs::read(&output).unwrap(), b"encoded");
    fs::remove_file(output).unwrap();
}

#[tokio::test]
async fn continuous_progress_does_not_extend_the_process_deadline() {
    let tool = Tool::new("while :; do printf 'frame=1\nprogress=continue\n'; done").await;
    let started = Instant::now();
    let mut records = 0;
    let ffmpeg = Ffmpeg::new(&tool.path, Some(Duration::from_millis(100)));
    let plan = tool.plan().await;
    let error = ffmpeg
        .build(&plan)
        .unwrap()
        .run(|_| records += 1, |_| {})
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
