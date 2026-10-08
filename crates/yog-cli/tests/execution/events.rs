use super::support::{Fixture, read_report};
use std::{fs, sync::Mutex, time::Duration};

#[tokio::test]
async fn recursive_events_count_all_candidates_and_each_kind_of_skipped_input() {
    let fixture = Fixture::new("for last do :; done; printf encoded > \"$last\"");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/movie.mkv"), b"video").unwrap();
    fs::write(fixture.0.join("input/audio.mka"), b"audio").unwrap();
    fs::write(fixture.0.join("input/notes.txt"), b"notes").unwrap();
    fs::write(fixture.0.join("input/cover.jpg"), b"image").unwrap();
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
streams='[{"index":0,"codec_type":"video"}]'
format=matroska,webm
case "$last" in
    *.txt) printf NOT-MEDIA >&2; exit 1 ;;
    *.mka) streams='[{"index":0,"codec_type":"audio"}]' ;;
    *.jpg) format=image2 ;;
esac
printf '{"streams":%s,"format":{"format_name":"%s","duration":"1"}}' "$streams" "$format""#,
    );
    let events = Mutex::new(Vec::new());

    let outcome = yog_runtime::run_with_events(
        yog_runtime::Command {
            request: yog_core::ffmpeg::plan::TranscodeRequest::new(
                fixture.0.join("input"),
                fixture.0.join("output"),
            ),
            operation: yog_runtime::Operation::Transcode,
            recursive: true,
        },
        yog_runtime::Options {
            ffmpeg: fixture.0.join("ffmpeg"),
            ffprobe: fixture.0.join("ffprobe"),
            terminal_output: false,
            report: Some(fixture.0.join("results.jsonl")),
            ..yog_runtime::Options::default()
        },
        yog_runtime::Config::default(),
        tokio_util::sync::CancellationToken::new(),
        |event| events.lock().unwrap().push(event),
    )
    .await;

    let yog_runtime::RunOutcome::Batch(batch) = outcome else {
        panic!("expected a batch outcome, got {outcome:?}");
    };
    assert_eq!(batch.total, 4);
    assert_eq!(batch.succeeded, 1);
    assert_eq!(batch.skipped, 3);
    assert!(batch.failures.is_empty());
    let records = fs::read_to_string(fixture.0.join("results.jsonl")).unwrap();
    let rows: Vec<serde_json::Value> = records
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let audio = rows
        .iter()
        .find_map(|row| {
            row.get("skipped")
                .filter(|record| record["input"].as_str().unwrap().ends_with("audio.mka"))
        })
        .unwrap();
    assert_eq!(audio["result"]["status"], "skipped");
    assert_eq!(audio["result"]["reason"], "no_video");
    let events = events.into_inner().unwrap();
    assert!(matches!(
        events
            .iter()
            .find(|event| matches!(event, yog_runtime::event::RunEvent::BatchDiscovered { .. })),
        Some(yog_runtime::event::RunEvent::BatchDiscovered { total: 4, .. })
    ));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                yog_runtime::event::RunEvent::TaskFinished {
                    result: yog_runtime::event::TaskResult::Skipped { .. },
                    ..
                }
            ))
            .count(),
        3
    );
}

#[tokio::test]
async fn cancelling_before_the_first_batch_probe_does_not_probe_later_files() {
    let fixture = Fixture::new("exit 99");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/first.mkv"), b"video").unwrap();
    fs::write(fixture.0.join("input/second.mkv"), b"video").unwrap();
    fixture.tool(
        "ffprobe",
        &format!(
            "touch '{}'\nprintf '%s' '{{\"streams\":[{{\"index\":0,\"codec_type\":\"video\"}}],\"format\":{{\"format_name\":\"matroska,webm\"}}}}'",
            fixture.0.join("probe-started").display(),
        ),
    );
    let cancellation = tokio_util::sync::CancellationToken::new();
    let events = Mutex::new(Vec::new());

    let outcome = yog_runtime::run_with_events(
        yog_runtime::Command {
            request: yog_core::ffmpeg::plan::TranscodeRequest::new(fixture.0.join("input"), "")
                .with_video(yog_core::ffmpeg::plan::VideoAction::encode_x264(None, None)),
            operation: yog_runtime::Operation::Predict,
            recursive: true,
        },
        yog_runtime::Options {
            ffmpeg: fixture.0.join("ffmpeg"),
            ffprobe: fixture.0.join("ffprobe"),
            terminal_output: false,
            report: Some(fixture.0.join("cancelled.jsonl")),
            ..yog_runtime::Options::default()
        },
        yog_runtime::Config::default(),
        cancellation.clone(),
        |event| {
            if matches!(event, yog_runtime::event::RunEvent::TaskStarted { .. }) {
                cancellation.cancel();
            }
            events.lock().unwrap().push(event);
        },
    )
    .await;

    let yog_runtime::RunOutcome::Batch(batch) = outcome else {
        panic!("expected a batch outcome, got {outcome:?}");
    };
    assert_eq!(batch.total, 2);
    assert_eq!(batch.succeeded, 0);
    assert_eq!(batch.skipped, 0);
    assert!(batch.cancelled);
    assert!(!fixture.0.join("probe-started").exists());
    assert!(!fixture.0.join("output").exists());
    let records = read_report(&fixture, "cancelled.jsonl");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["predict"]["result"]["status"], "cancelled");
    let completions: Vec<_> = events
        .into_inner()
        .unwrap()
        .into_iter()
        .filter_map(|event| match event {
            yog_runtime::event::RunEvent::TaskFinished { result, .. } => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(completions, [yog_runtime::event::TaskResult::Cancelled]);
}

#[tokio::test]
async fn preflight_skip_has_one_completion_without_a_fake_start() {
    use yog_runtime::event::{RunEvent, SkipReason, TaskResult};
    let fixture = Fixture::new("for last do :; done; printf encoded > \"$last\"");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/movie.txt"), b"notes").unwrap();
    fs::write(fixture.0.join("input/movie.mkv"), b"video").unwrap();
    fixture.tool("ffprobe", r#"for last do :; done
case "$last" in *.txt) exit 1 ;; esac
printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"matroska,webm","duration":"1"}}'"#);
    let mut request = yog_core::ffmpeg::plan::TranscodeRequest::new(
        fixture.0.join("input"),
        fixture.0.join("output"),
    );
    request.container = Some(yog_core::ffmpeg::plan::Container::Matroska);
    let events = Mutex::new(Vec::new());
    let outcome = yog_runtime::run_with_events(
        yog_runtime::Command {
            request,
            operation: yog_runtime::Operation::Transcode,
            recursive: true,
        },
        yog_runtime::Options {
            ffmpeg: fixture.0.join("ffmpeg"),
            ffprobe: fixture.0.join("ffprobe"),
            terminal_output: false,
            report: Some(fixture.0.join("report.jsonl")),
            ..yog_runtime::Options::default()
        },
        yog_runtime::Config::default(),
        tokio_util::sync::CancellationToken::new(),
        |event| events.lock().unwrap().push(event),
    )
    .await;
    let yog_runtime::RunOutcome::Batch(batch) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(batch.total, 2);
    assert_eq!(batch.succeeded, 1);
    assert_eq!(batch.skipped, 1);
    assert!(batch.failures.is_empty());
    let events = events.into_inner().unwrap();
    let skipped_path = fixture.0.join("input/movie.txt");
    assert!(!events.iter().any(
        |event| matches!(event, RunEvent::TaskStarted { input, .. } if input == &skipped_path)
    ));
    let completions: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            RunEvent::TaskFinished { input, result, .. } if input == &skipped_path => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(completions.len(), 1);
    assert!(matches!(
        completions[0],
        TaskResult::Skipped {
            reason: SkipReason::ProbeFailed,
            ..
        }
    ));
    let rows = read_report(&fixture, "report.jsonl");
    assert_eq!(rows.len(), 2);
    let skipped = rows.iter().find_map(|row| row.get("skipped")).unwrap();
    assert_eq!(
        skipped["result"],
        serde_json::to_value(completions[0]).unwrap()
    );
}

#[tokio::test]
async fn probe_timeout_is_a_skip_not_a_transcode_failure() {
    use yog_runtime::event::{RunEvent, SkipReason, TaskResult};
    let fixture = Fixture::new("touch ffmpeg-started");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/stalled.mkv"), b"video").unwrap();
    fixture.tool("ffprobe", "while :; do :; done");
    let events = Mutex::new(Vec::new());
    let outcome = yog_runtime::run_with_events(
        yog_runtime::Command {
            request: yog_core::ffmpeg::plan::TranscodeRequest::new(
                fixture.0.join("input"),
                fixture.0.join("output"),
            ),
            operation: yog_runtime::Operation::Transcode,
            recursive: true,
        },
        yog_runtime::Options {
            ffmpeg: fixture.0.join("ffmpeg"),
            ffprobe: fixture.0.join("ffprobe"),
            timeout: Some(Duration::from_millis(100)),
            terminal_output: false,
            report: Some(fixture.0.join("report.jsonl")),
            ..yog_runtime::Options::default()
        },
        yog_runtime::Config::default(),
        tokio_util::sync::CancellationToken::new(),
        |event| events.lock().unwrap().push(event),
    )
    .await;
    let yog_runtime::RunOutcome::Batch(batch) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(batch.total, 1);
    assert_eq!(batch.skipped, 1);
    assert!(batch.failures.is_empty());
    assert!(!fixture.0.join("ffmpeg-started").exists());
    let completions: Vec<_> = events
        .into_inner()
        .unwrap()
        .into_iter()
        .filter_map(|event| match event {
            RunEvent::TaskFinished { result, .. } => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(completions.len(), 1);
    assert!(matches!(
        completions[0],
        TaskResult::Skipped {
            reason: SkipReason::ProbeTimedOut,
            ..
        }
    ));
    assert_eq!(
        read_report(&fixture, "report.jsonl")[0]["skipped"]["result"],
        serde_json::to_value(&completions[0]).unwrap()
    );
}
