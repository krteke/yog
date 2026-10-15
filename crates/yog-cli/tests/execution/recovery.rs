use super::support::Fixture;
use std::fs;

fn batch() -> Fixture {
    let fixture = Fixture::new(
        r#"printf 'encode\n' >> encoded-files
for last do :; done
printf encoded > "$last""#,
    );
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/movie.mkv"), b"video").unwrap();
    fixture.tool("ffprobe", r#"printf 'probe\n' >> probed-files
printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"matroska,webm","duration":"1"}}'"#);
    fixture
}

#[test]
fn automatic_recovery_reuses_completed_files_without_probing() {
    let fixture = batch();
    let first = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();
    assert!(first.status.success(), "{:?}", first.stderr);
    let second = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();
    assert!(second.status.success(), "{:?}", second.stderr);
    assert_eq!(
        fs::read_to_string(fixture.0.join("encoded-files")).unwrap(),
        "encode\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("probed-files")).unwrap(),
        "probe\n"
    );
    assert!(String::from_utf8_lossy(&second.stdout).contains("reused 1"));
    assert!(fixture.0.join("output/.yog-state.jsonl").exists());
}

#[test]
fn confirmation_without_a_terminal_preserves_state_and_explains_unattended_options() {
    let fixture = batch();
    let first = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();
    assert!(first.status.success(), "{:?}", first.stderr);
    let path = fixture.0.join("output/.yog-state.jsonl");
    let saved = fs::read(&path).unwrap();
    fs::write(fixture.0.join("input/movie.mkv"), b"changed source").unwrap();
    let result = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);

    assert!(!result.status.success());
    assert!(stderr.contains("interactive terminal"), "{stderr}");
    assert!(stderr.contains("--resume-file-mismatch"), "{stderr}");
    assert_eq!(fs::read(path).unwrap(), saved);
    assert_eq!(
        fs::read(fixture.0.join("output/movie.mkv")).unwrap(),
        b"encoded"
    );
}

#[test]
fn force_recovery_keeps_completed_outputs_in_the_old_directory() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("old")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(fixture.0.join("input/new.mkv"), b"new").unwrap();
    let result = fixture
        .recursive_command("new")
        .args([
            "--copy",
            "--state",
            "old/.yog-state.jsonl",
            "--resume-batch-mismatch",
            "force",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(fixture.0.join("old/movie.mkv").exists());
    assert!(!fixture.0.join("new/movie.mkv").exists());
    assert!(fixture.0.join("new/new.mkv").exists());
    assert_eq!(
        fs::read_to_string(fixture.0.join("encoded-files")).unwrap(),
        "encode\nencode\n"
    );
}

#[test]
fn file_mismatch_abort_is_resolved_before_any_transcode() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(fixture.0.join("input/movie.mkv"), b"changed source").unwrap();
    fs::write(fixture.0.join("input/new.mkv"), b"new").unwrap();
    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--resume-file-mismatch", "abort"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("not processed 2 | recovery error"));
    assert!(!fixture.0.join("output/new.mkv").exists());
    assert_eq!(
        fs::read_to_string(fixture.0.join("encoded-files")).unwrap(),
        "encode\n"
    );
}

#[test]
fn recovery_reexecutes_changed_sources_without_granting_output_overwrite() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(fixture.0.join("input/movie.mkv"), b"changed source").unwrap();
    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--resume-file-mismatch", "restart"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("output file already exists"));
    assert_eq!(
        fs::read(fixture.0.join("output/movie.mkv")).unwrap(),
        b"encoded"
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("encoded-files")).unwrap(),
        "encode\n"
    );
}

#[test]
fn restarting_state_does_not_grant_output_overwrite() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--restart"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        fs::read(fixture.0.join("output/movie.mkv")).unwrap(),
        b"encoded"
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("encoded-files")).unwrap(),
        "encode\n"
    );
}

#[test]
fn no_state_disables_recovery_and_does_not_write_a_journal() {
    let fixture = batch();
    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--no-state"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(!fixture.0.join("output/.yog-state.jsonl").exists());
}

#[test]
fn batch_mismatch_skip_preserves_old_records_and_processes_new_files() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("old")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(fixture.0.join("input/new.mkv"), b"new").unwrap();
    let result = fixture
        .recursive_command("new")
        .args([
            "--copy",
            "--state",
            "old/.yog-state.jsonl",
            "--resume-batch-mismatch",
            "skip",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(!fixture.0.join("new/movie.mkv").exists());
    assert!(fixture.0.join("new/new.mkv").exists());
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains("succeeded 1 | failed 0 | skipped 1 | reused 0")
    );
}

#[test]
fn explicit_restart_can_use_the_old_output_directory_as_a_new_input() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fixture.tool("ffprobe", r#"for last do :; done
case "$last" in *.jsonl) exit 1 ;; esac
printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"matroska,webm","duration":"1"}}'"#);
    let result = fixture
        .cli()
        .args([
            "transcode",
            "-i",
            "output",
            "-o",
            "new",
            "--recursive",
            "--copy",
            "--ffmpeg",
            "./ffmpeg",
            "--ffprobe",
            "./ffprobe",
            "--state",
            "output/.yog-state.jsonl",
            "--restart",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(fixture.0.join("new/movie.mkv").exists());
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains("total 2 | succeeded 1 | failed 0 | skipped 1")
    );
}

#[test]
fn file_force_explicitly_trusts_a_record_even_when_the_output_is_missing() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::remove_file(fixture.0.join("output/movie.mkv")).unwrap();
    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--resume-file-mismatch", "force"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(String::from_utf8_lossy(&result.stdout).contains("reused 1 | forced 1"));
    assert!(String::from_utf8_lossy(&result.stderr).contains("after accepting differences"));
    assert_eq!(
        fs::read_to_string(fixture.0.join("encoded-files")).unwrap(),
        "encode\n"
    );
    assert!(!fixture.0.join("output/movie.mkv").exists());
}

#[test]
fn mismatch_skip_does_not_replace_the_previous_success_record() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(fixture.0.join("input/movie.mkv"), b"changed source").unwrap();
    let skipped = fixture
        .recursive_command("output")
        .args(["--copy", "--resume-file-mismatch", "skip"])
        .output()
        .unwrap();
    assert!(skipped.status.success(), "{:?}", skipped.stderr);
    assert!(String::from_utf8_lossy(&skipped.stdout).contains("skipped 1"));
    let records = super::support::read_report(&fixture, "output/.yog-state.jsonl");
    assert_eq!(records.last().unwrap()["result"]["status"], "success");
    let forced = fixture
        .recursive_command("output")
        .args(["--copy", "--resume-file-mismatch", "force"])
        .output()
        .unwrap();
    assert!(forced.status.success(), "{:?}", forced.stderr);
    assert!(String::from_utf8_lossy(&forced.stdout).contains("reused 1 | forced 1"));
}

#[test]
fn recovered_success_still_protects_against_output_mapping_collisions() {
    let fixture = batch();
    assert!(
        fixture
            .recursive_command("output")
            .args(["--copy", "--container", "mkv"])
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(fixture.0.join("input/movie.mp4"), b"second video").unwrap();
    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--container", "mkv", "--overwrite-output"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("multiple input files map"));
    assert_eq!(
        fs::read_to_string(fixture.0.join("encoded-files")).unwrap(),
        "encode\n"
    );
}

#[test]
fn deterministic_non_video_skip_is_reused_without_probing() {
    let fixture = batch();
    fixture.tool(
        "ffprobe",
        r#"printf 'probe\n' >> probed-files
printf '%s' '{"streams":[],"format":{"format_name":"matroska,webm"}}'"#,
    );
    assert!(
        !fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    let result = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        fs::read_to_string(fixture.0.join("probed-files")).unwrap(),
        "probe\n"
    );
}

#[test]
fn probe_failure_is_retried_instead_of_becoming_a_permanent_skip() {
    let fixture = batch();
    fixture.tool("ffprobe", "printf 'probe\n' >> probed-files\nexit 1");
    assert!(
        !fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fixture.tool("ffprobe", r#"printf 'probe\n' >> probed-files
printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"matroska,webm","duration":"1"}}'"#);
    let result = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(
        fs::read_to_string(fixture.0.join("probed-files")).unwrap(),
        "probe\nprobe\n"
    );
}

#[test]
fn failed_transcode_is_retried_without_reencoding_other_completed_files() {
    let fixture = batch();
    fs::write(fixture.0.join("input/fail.mkv"), b"video").unwrap();
    fixture.tool(
        "ffmpeg",
        r#"for last do :; done
case "$last" in *fail*) exit 1 ;; esac
printf 'encode\n' >> encoded-files
printf encoded > "$last""#,
    );
    assert!(
        !fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fixture.tool(
        "ffmpeg",
        "for last do :; done\nprintf 'encode\n' >> encoded-files\nprintf encoded > \"$last\"",
    );
    let result = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(String::from_utf8_lossy(&result.stdout).contains("reused 1"));
    assert_eq!(
        fs::read_to_string(fixture.0.join("encoded-files")).unwrap(),
        "encode\nencode\n"
    );
}

#[test]
fn file_mismatch_skip_also_applies_to_a_previously_failed_file() {
    let fixture = batch();
    fixture.tool("ffmpeg", "exit 1");
    assert!(
        !fixture
            .recursive_command("output")
            .arg("--copy")
            .output()
            .unwrap()
            .status
            .success()
    );
    fs::write(fixture.0.join("input/movie.mkv"), b"changed source").unwrap();
    fixture.tool("ffmpeg", "for last do :; done\nprintf encoded > \"$last\"");
    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--resume-file-mismatch", "skip"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert!(!fixture.0.join("output/movie.mkv").exists());
    assert!(String::from_utf8_lossy(&result.stdout).contains("succeeded 0 | failed 0 | skipped 1"));
}

#[test]
fn report_overwrite_permission_does_not_allow_overwriting_a_video_output() {
    let fixture = batch();
    fs::create_dir(fixture.0.join("output")).unwrap();
    fs::write(fixture.0.join("output/movie.mkv"), b"previous video").unwrap();
    let result = fixture
        .recursive_command("output")
        .args([
            "--copy",
            "--report",
            "output/movie.mkv",
            "--overwrite-report",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        fs::read(fixture.0.join("output/movie.mkv")).unwrap(),
        b"previous video"
    );
    assert!(!fixture.0.join("encoded-files").exists());
}

fn runtime_batch(fixture: &Fixture) -> (yog_runtime::Command, yog_runtime::Options) {
    fixture.tool("ffmpeg", "for last do :; done\nprintf encoded > \"$last\"");
    fixture.tool("ffprobe", r#"printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"matroska,webm","duration":"1"}}'"#);
    (
        yog_runtime::Command {
            request: yog_core::ffmpeg::plan::TranscodeRequest::new(
                fixture.0.join("input"),
                fixture.0.join("output"),
            )
            .with_video(yog_core::ffmpeg::plan::VideoAction::Copy),
            operation: yog_runtime::Operation::Transcode,
            recursive: true,
        },
        yog_runtime::Options {
            ffmpeg: fixture.0.join("ffmpeg"),
            ffprobe: fixture.0.join("ffprobe"),
            terminal_output: false,
            ..yog_runtime::Options::default()
        },
    )
}

#[tokio::test]
async fn cancellation_during_recovery_preflight_takes_precedence_over_file_mismatch() {
    let fixture = batch();
    let (command, options) = runtime_batch(&fixture);
    let first = yog_runtime::run(
        command,
        options,
        yog_runtime::Config::default(),
        yog_runtime::control::RunControl::new(),
    )
    .await;
    assert_eq!(first.status(), yog_runtime::RunStatus::Success);
    fs::write(fixture.0.join("input/movie.mkv"), b"changed source").unwrap();
    let (command, mut options) = runtime_batch(&fixture);
    options.recovery.file_mismatch = yog_runtime::recovery::MismatchPolicy::Abort;
    let cancellation = yog_runtime::control::RunControl::new();
    let outcome = yog_runtime::run_with_events(
        command,
        options,
        yog_runtime::Config::default(),
        cancellation.clone(),
        |event| {
            if matches!(event, yog_runtime::event::RunEvent::BatchDiscovered { .. }) {
                cancellation.cancel();
            }
        },
    )
    .await;
    let yog_runtime::RunOutcome::Batch(batch) = outcome else {
        panic!("expected a cancelled batch, got {outcome:?}");
    };
    assert!(batch.cancelled);
    assert_eq!(batch.total, 1);
    assert_eq!(batch.succeeded, 0);
    assert_eq!(
        fs::read(fixture.0.join("output/movie.mkv")).unwrap(),
        b"encoded"
    );
}

#[tokio::test]
async fn recovered_success_emits_one_completion_without_a_fake_start_and_is_reported() {
    let fixture = batch();
    let (command, options) = runtime_batch(&fixture);
    let first = yog_runtime::run(
        command,
        options,
        yog_runtime::Config::default(),
        yog_runtime::control::RunControl::new(),
    )
    .await;
    assert_eq!(first.status(), yog_runtime::RunStatus::Success);
    let (command, mut options) = runtime_batch(&fixture);
    options.report = Some(fixture.0.join("recovery-report.jsonl"));
    let events = std::sync::Mutex::new(Vec::new());
    let outcome = yog_runtime::run_with_events(
        command,
        options,
        yog_runtime::Config::default(),
        yog_runtime::control::RunControl::new(),
        |event| events.lock().unwrap().push(event),
    )
    .await;
    assert_eq!(outcome.status(), yog_runtime::RunStatus::Success);
    let events = events.into_inner().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, yog_runtime::event::RunEvent::TaskStarted { .. }))
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                yog_runtime::event::RunEvent::TaskFinished {
                    recovery: Some(_),
                    ..
                }
            ))
            .count(),
        1
    );
    let records = super::support::read_report(&fixture, "recovery-report.jsonl");
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0]["skipped"]["result"]["reason"],
        "already_completed"
    );
    assert_eq!(records[0]["skipped"]["recovery"]["forced"], false);
    assert_eq!(
        records[0]["skipped"]["recovery"]["output"],
        fixture.0.join("output/movie.mkv").to_str().unwrap()
    );
}
