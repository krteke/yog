use super::support::{Fixture, read_report};
use std::{
    fs,
    num::NonZeroUsize,
    process::{Command as Process, Stdio},
    sync::Mutex,
    time::Duration,
};
use yog_core::ffmpeg::plan::TranscodeRequest;
use yog_runtime::{
    Command, Config, Operation, Options, RunOutcome, RunStatus,
    control::RunControl,
    event::{RunEvent, SkipReason, TaskResult},
};

fn fixture(ffmpeg: &str, files: usize) -> Fixture {
    let fixture = Fixture::new("exit 99");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::create_dir(fixture.0.join("output")).unwrap();
    for index in 0..files {
        fs::write(fixture.0.join(format!("input/{index}.mkv")), b"source").unwrap();
        fs::write(fixture.0.join(format!("output/{index}.mkv")), b"previous").unwrap();
    }
    fixture.tool("ffmpeg", &format!("cd '{}'\n{ffmpeg}", fixture.0.display()));
    fixture.tool("ffprobe", &format!("cd '{}'\nprintf 'probe\\n' >> probes\nprintf '%s' '{{\"streams\":[{{\"index\":0,\"codec_type\":\"video\"}}],\"format\":{{\"format_name\":\"matroska,webm\",\"duration\":\"1\"}}}}'", fixture.0.display()));
    fixture
}

fn command(fixture: &Fixture) -> Command {
    Command {
        request: TranscodeRequest::new(fixture.0.join("input"), fixture.0.join("output"))
            .with_overwrite_output(true),
        operation: Operation::Transcode,
        recursive: true,
    }
}

fn options(fixture: &Fixture) -> Options {
    Options {
        ffmpeg: fixture.0.join("ffmpeg"),
        ffprobe: fixture.0.join("ffprobe"),
        timeout: Some(Duration::from_secs(3)),
        terminal_output: false,
        report: Some(fixture.0.join("report.jsonl")),
        ..Options::default()
    }
}

#[tokio::test]
async fn restored_completion_does_not_consume_a_manual_skip() {
    let fixture = fixture("for last do :; done\nprintf encoded > \"$last\"", 4);
    let first = yog_runtime::run(
        command(&fixture),
        options(&fixture),
        Config::default(),
        RunControl::new(),
    )
    .await;
    assert_eq!(first.status(), RunStatus::Success, "{first:?}");
    let rows = read_report(&fixture, "report.jsonl");
    let restored_output = serde_json::json!({
        "Unix": rows[1]["transcode"]["output"].as_str().unwrap().as_bytes()
    });
    let path = fixture.0.join("output/.yog-state.jsonl");
    let state = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|line| {
            let entry: serde_json::Value = serde_json::from_str(line).unwrap();
            entry.get("result").is_none() || entry["output"]["path"] == restored_output
        })
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    fs::write(path, state).unwrap();
    let control = RunControl::new();
    let mut options = options(&fixture);
    options.overwrite_report = true;
    let outcome = yog_runtime::run_with_events(
        command(&fixture),
        options,
        Config::default(),
        control.clone(),
        |event| {
            if matches!(event, RunEvent::TaskStarted { index: 1, .. }) {
                assert!(control.skip_tasks(1, NonZeroUsize::new(2).unwrap()));
            }
        },
    )
    .await;
    assert_eq!(outcome.status(), RunStatus::Success, "{outcome:?}");
    let RunOutcome::Batch(batch) = outcome else {
        panic!("expected batch")
    };
    assert_eq!(batch.manual_skipped, 2);
    assert_eq!(batch.reused, 1);
    assert_eq!(batch.succeeded, 1);
    let rows = read_report(&fixture, "report.jsonl");
    assert_eq!(rows[1]["skipped"]["result"]["reason"], "already_completed");
    assert_eq!(rows[2]["skipped"]["result"]["reason"], "manual");
    assert_eq!(rows[3]["transcode"]["result"]["status"], "success");
}

#[tokio::test]
async fn skipping_one_encode_preserves_encoder_help_for_the_next_tasks() {
    let fixture = fixture(
        r#"case "$*" in
    *encoder=libx264*)
        test ! -e help-read || exit 88
        touch help-read
        printf 'Encoder libx264 [H.264]:\n    Supported pixel formats: yuv420p\n'
        exit 0 ;;
esac
for last do :; done
printf encoded > "$last""#,
        3,
    );
    fixture.tool("ffprobe", "printf '%s' '{\"streams\":[{\"index\":0,\"codec_type\":\"video\",\"pix_fmt\":\"yuv420p\",\"width\":32,\"height\":32}],\"format\":{\"format_name\":\"matroska,webm\",\"duration\":\"1\"},\"pixel_formats\":[{\"name\":\"yuv420p\",\"nb_components\":3,\"log2_chroma_w\":1,\"log2_chroma_h\":1,\"flags\":{\"rgb\":0,\"alpha\":0,\"palette\":0,\"hwaccel\":0},\"components\":[{\"bit_depth\":8},{\"bit_depth\":8},{\"bit_depth\":8}]}]}'");
    let mut command = command(&fixture);
    command.request.video = yog_core::ffmpeg::plan::VideoAction::encode_x264(None, None);
    let control = RunControl::new();
    let current = Mutex::new(0);
    let outcome = yog_runtime::run_with_events(
        command,
        options(&fixture),
        Config::default(),
        control.clone(),
        |event| match event {
            RunEvent::TaskStarted { index, .. } => *current.lock().unwrap() = index,
            RunEvent::PhaseChanged(yog_runtime::event::RunPhase::Transcoding)
                if *current.lock().unwrap() == 1 =>
            {
                assert!(control.skip_tasks(1, NonZeroUsize::new(1).unwrap()));
            }
            _ => {}
        },
    )
    .await;
    assert_eq!(outcome.status(), RunStatus::Success, "{outcome:?}");
    let RunOutcome::Batch(batch) = outcome else {
        panic!("expected batch")
    };
    assert_eq!(batch.succeeded, 2);
    assert_eq!(batch.manual_skipped, 1);
    assert!(fixture.0.join("help-read").exists());
}

#[tokio::test]
async fn skipping_more_than_the_remaining_tasks_finishes_without_no_video_failure() {
    let fixture = fixture("exit 99", 2);
    let control = RunControl::new();
    let outcome = yog_runtime::run_with_events(
        command(&fixture),
        options(&fixture),
        Config::default(),
        control.clone(),
        |event| {
            if matches!(event, RunEvent::TaskStarted { index: 1, .. }) {
                assert!(control.skip_tasks(1, NonZeroUsize::new(20).unwrap()));
            }
        },
    )
    .await;
    assert_eq!(outcome.status(), RunStatus::Success, "{outcome:?}");
    let RunOutcome::Batch(batch) = outcome else {
        panic!("expected batch")
    };
    assert_eq!(batch.manual_skipped, 2);
    assert_eq!(batch.skipped, 2);
    assert!(batch.failures.is_empty());
    assert!(!fixture.0.join("probes").exists());
}

#[tokio::test]
async fn skipping_during_verification_does_not_publish_the_unverified_part() {
    let fixture = fixture("for last do :; done\nprintf encoded > \"$last\"", 2);
    let control = RunControl::new();
    let mut options = options(&fixture);
    options.verify = true;
    let current = Mutex::new(0);
    let output = Mutex::new(None);
    let outcome = yog_runtime::run_with_events(
        command(&fixture),
        options,
        Config::default(),
        control.clone(),
        |event| match event {
            RunEvent::TaskStarted {
                index,
                output: target,
                ..
            } => {
                *current.lock().unwrap() = index;
                if index == 1 {
                    *output.lock().unwrap() = target;
                }
            }
            RunEvent::PhaseChanged(yog_runtime::event::RunPhase::Verifying)
                if *current.lock().unwrap() == 1 =>
            {
                assert!(control.skip_tasks(1, NonZeroUsize::new(1).unwrap()));
            }
            _ => {}
        },
    )
    .await;
    assert_eq!(outcome.status(), RunStatus::Success, "{outcome:?}");
    let RunOutcome::Batch(batch) = outcome else {
        panic!("expected batch")
    };
    assert_eq!(batch.succeeded, 1);
    assert_eq!(batch.manual_skipped, 1);
    assert_eq!(
        fs::read(output.into_inner().unwrap().unwrap()).unwrap(),
        b"previous"
    );
    let rows = read_report(&fixture, "report.jsonl");
    assert_eq!(rows[0]["transcode"]["result"]["reason"], "manual");
}

#[tokio::test]
async fn skipping_current_video_reaps_the_process_and_preserves_its_previous_output() {
    let fixture = fixture(
        r#"for last do :; done
printf encoded > "$last"
if test ! -e attempted; then
    touch attempted
    printf '%s' "$$" > child-pid
    printf 'frame=1\nout_time_us=1\nprogress=continue\n'
    while :; do :; done
fi"#,
        3,
    );
    let control = RunControl::new();
    let first_output = Mutex::new(None);
    let results = Mutex::new(Vec::new());
    let outcome = yog_runtime::run_with_events(
        command(&fixture),
        options(&fixture),
        Config::default(),
        control.clone(),
        |event| match event {
            RunEvent::TaskStarted {
                index: 1, output, ..
            } => *first_output.lock().unwrap() = output,
            RunEvent::Progress { .. } => {
                assert!(control.skip_tasks(1, NonZeroUsize::new(1).unwrap()));
            }
            RunEvent::TaskFinished { result, .. } => results.lock().unwrap().push(result),
            _ => {}
        },
    )
    .await;
    assert_eq!(outcome.status(), RunStatus::Success, "{outcome:?}");
    let RunOutcome::Batch(batch) = outcome else {
        panic!("expected batch")
    };
    assert_eq!(batch.succeeded, 2);
    assert_eq!(batch.manual_skipped, 1);
    assert!(!control.is_cancelled());
    assert!(matches!(
        &results.lock().unwrap()[0],
        TaskResult::Skipped {
            reason: SkipReason::Manual,
            ..
        }
    ));
    let output = first_output.into_inner().unwrap().unwrap();
    assert_eq!(fs::read(&output).unwrap(), b"previous");
    assert!(
        !output
            .with_file_name(format!(
                ".part.{}",
                output.file_name().unwrap().to_str().unwrap()
            ))
            .exists()
    );
    let pid = fs::read_to_string(fixture.0.join("child-pid")).unwrap();
    assert!(
        !Process::new("kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let rows = read_report(&fixture, "report.jsonl");
    assert_eq!(rows[0]["transcode"]["result"]["reason"], "manual");
    let state: Vec<serde_json::Value> =
        fs::read_to_string(fixture.0.join("output/.yog-state.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    let skipped = state
        .iter()
        .rev()
        .find(|entry| entry["result"]["reason"] == "manual")
        .unwrap();
    assert_eq!(skipped["part"], false);
}

#[tokio::test]
async fn skipping_two_tasks_includes_current_and_does_not_probe_the_following_task() {
    let fixture = fixture("for last do :; done\nprintf encoded > \"$last\"", 4);
    let control = RunControl::new();
    let outcome = yog_runtime::run_with_events(
        command(&fixture),
        options(&fixture),
        Config::default(),
        control.clone(),
        |event| {
            if matches!(event, RunEvent::TaskStarted { index: 1, .. }) {
                assert!(control.skip_tasks(1, NonZeroUsize::new(2).unwrap()));
            }
        },
    )
    .await;
    assert_eq!(outcome.status(), RunStatus::Success, "{outcome:?}");
    let RunOutcome::Batch(batch) = outcome else {
        panic!("expected batch")
    };
    assert_eq!(batch.succeeded, 2);
    assert_eq!(batch.manual_skipped, 2);
    assert_eq!(
        fs::read_to_string(fixture.0.join("probes")).unwrap(),
        "probe\nprobe\n"
    );
    let rows = read_report(&fixture, "report.jsonl");
    assert_eq!(rows[0]["transcode"]["result"]["reason"], "manual");
    assert_eq!(rows[1]["skipped"]["result"]["reason"], "manual");
}

#[tokio::test]
async fn skipping_published_video_only_skips_vmaf_and_keeps_success_in_state() {
    let fixture = fixture("for last do :; done\nprintf encoded > \"$last\"", 2);
    fixture.tool("ffprobe", "printf '%s' '{\"streams\":[{\"index\":0,\"codec_type\":\"video\",\"width\":32,\"height\":32}],\"format\":{\"format_name\":\"matroska,webm\",\"duration\":\"1\"}}'");
    let control = RunControl::new();
    let mut options = options(&fixture);
    options.vmaf = Some(yog_core::ffmpeg::vmaf::VmafOptions::default());
    let outcome = yog_runtime::run_with_events(
        command(&fixture),
        options,
        Config::default(),
        control.clone(),
        |event| {
            if matches!(
                event,
                RunEvent::PhaseChanged(yog_runtime::event::RunPhase::CalculatingVmaf)
            ) {
                assert!(control.skip_tasks(1, NonZeroUsize::new(2).unwrap()));
            }
        },
    )
    .await;
    assert_eq!(outcome.status(), RunStatus::Success, "{outcome:?}");
    let RunOutcome::Batch(batch) = outcome else {
        panic!("expected batch")
    };
    assert_eq!(batch.succeeded, 1);
    assert_eq!(batch.manual_skipped, 1);
    let rows = read_report(&fixture, "report.jsonl");
    assert_eq!(rows[0]["transcode"]["result"]["status"], "success");
    assert_eq!(rows[0]["transcode"]["vmaf"]["skip_reason"], "manual");
    assert!(rows[0]["transcode"]["vmaf"]["error"].is_null());
    let output = rows[0]["transcode"]["output"].as_str().unwrap();
    assert_eq!(fs::read(output).unwrap(), b"encoded");
    let state: Vec<serde_json::Value> =
        fs::read_to_string(fixture.0.join("output/.yog-state.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert!(
        state
            .iter()
            .any(|entry| entry["result"]["status"] == "success")
    );
}

#[tokio::test]
async fn cancelling_batch_discards_the_unconsumed_manual_skip_quota() {
    let fixture = fixture("exit 99", 3);
    let control = RunControl::new();
    let outcome = yog_runtime::run_with_events(
        command(&fixture),
        options(&fixture),
        Config::default(),
        control.clone(),
        |event| match event {
            RunEvent::TaskStarted { index: 1, .. } => {
                assert!(control.skip_tasks(1, NonZeroUsize::new(3).unwrap()));
            }
            RunEvent::TaskFinished { index: 1, .. } => control.cancel(),
            _ => {}
        },
    )
    .await;
    assert_eq!(outcome.status(), RunStatus::Cancelled, "{outcome:?}");
    let RunOutcome::Batch(batch) = outcome else {
        panic!("expected batch")
    };
    assert_eq!(batch.manual_skipped, 1);
    assert_eq!(batch.skipped, 1);
    assert_eq!(read_report(&fixture, "report.jsonl").len(), 1);
    let state: Vec<serde_json::Value> =
        fs::read_to_string(fixture.0.join("output/.yog-state.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(
        state.len(),
        3,
        "only the header and current task's two records are saved"
    );
    assert!(!fixture.0.join("probes").exists());
}
