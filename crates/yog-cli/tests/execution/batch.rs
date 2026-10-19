use super::support::{Fixture, read_report};
use std::fs;

#[test]
fn an_all_existing_output_batch_succeeds_without_probing_or_overwriting() {
    let fixture = Fixture::new("touch ffmpeg-started; exit 9");
    fixture.tool("ffprobe", "touch ffprobe-started; exit 9");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::create_dir(fixture.0.join("output")).unwrap();
    fs::write(fixture.0.join("input/movie.mkv"), b"source").unwrap();
    fs::write(fixture.0.join("output/movie.mkv"), b"original").unwrap();

    let result = fixture
        .recursive_command("output")
        .args(["--no-state", "--report", "results.jsonl", "--copy"])
        .output()
        .unwrap();

    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(
        fs::read(fixture.0.join("output/movie.mkv")).unwrap(),
        b"original"
    );
    assert!(!fixture.0.join("ffprobe-started").exists());
    assert!(!fixture.0.join("ffmpeg-started").exists());
    assert!(!fixture.0.join("output/.part.movie.mkv").exists());
    assert!(!fixture.0.join("output/.yog-state.jsonl").exists());
    let records = read_report(&fixture, "results.jsonl");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["skipped"]["result"]["status"], "skipped");
    assert_eq!(records[0]["skipped"]["result"]["reason"], "existing_output");
    assert!(String::from_utf8_lossy(&result.stdout).contains("failed 0 | skipped 1"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("no video files found"));
}

#[test]
fn existing_outputs_are_recorded_as_skipped_while_new_tasks_are_transcoded() {
    let fixture = Fixture::new("for last do :; done; printf encoded > \"$last\"");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::create_dir(fixture.0.join("output")).unwrap();
    fs::write(fixture.0.join("input/done.mkv"), b"done").unwrap();
    fs::write(fixture.0.join("input/new.mkv"), b"new").unwrap();
    fs::write(fixture.0.join("output/done.mkv"), b"original").unwrap();

    let result = fixture
        .recursive_command("output")
        .args(["--report", "results.jsonl", "--copy"])
        .output()
        .unwrap();

    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(
        fs::read(fixture.0.join("output/done.mkv")).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read(fixture.0.join("output/new.mkv")).unwrap(),
        b"encoded"
    );
    let records = read_report(&fixture, "results.jsonl");
    assert_eq!(records.len(), 2);
    let skipped = records.iter().find_map(|row| row.get("skipped")).unwrap();
    assert_eq!(skipped["result"]["reason"], "existing_output");
    let state = read_report(&fixture, "output/.yog-state.jsonl");
    assert!(
        state
            .iter()
            .any(|entry| entry["result"]["reason"] == "existing_output")
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("succeeded 1 | failed 0 | skipped 1"));
}

#[test]
fn overwrite_output_reencodes_existing_batch_outputs() {
    let fixture = Fixture::new("for last do :; done; printf replacement > \"$last\"");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::create_dir(fixture.0.join("output")).unwrap();
    fs::write(fixture.0.join("input/movie.mkv"), b"source").unwrap();
    fs::write(fixture.0.join("output/movie.mkv"), b"original").unwrap();

    let result = fixture
        .recursive_command("output")
        .args(["--overwrite-output", "--copy"])
        .output()
        .unwrap();

    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(
        fs::read(fixture.0.join("output/movie.mkv")).unwrap(),
        b"replacement"
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("succeeded 1 | failed 0 | skipped 0"));
}

#[test]
fn an_unowned_batch_part_is_a_failure_not_an_existing_output_skip() {
    let fixture = Fixture::new("touch ffmpeg-started");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::create_dir(fixture.0.join("output")).unwrap();
    fs::write(fixture.0.join("input/movie.mkv"), b"source").unwrap();
    fs::write(fixture.0.join("output/.part.movie.mkv"), b"unfinished").unwrap();

    let result = fixture
        .recursive_command("output")
        .args(["--no-state", "--report", "results.jsonl", "--copy"])
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        fs::read(fixture.0.join("output/.part.movie.mkv")).unwrap(),
        b"unfinished"
    );
    assert!(!fixture.0.join("ffmpeg-started").exists());
    let records = read_report(&fixture, "results.jsonl");
    assert_eq!(records[0]["transcode"]["result"]["status"], "failed");
    assert_eq!(
        records[0]["transcode"]["result"]["phase"],
        "preparing_output"
    );
}

#[test]
fn recursive_mode_probes_each_file_just_before_transcoding_it() {
    let fixture = Fixture::new(
        r#"previous=''
for argument do
    if test "$previous" = -i; then printf 'transcode %s\n' "$argument" >> execution-order; fi
    previous=$argument
done
printf encoded > "$argument""#,
    );
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/first.mkv"), b"first").unwrap();
    fs::write(fixture.0.join("input/second.mkv"), b"second").unwrap();
    fs::write(fixture.0.join("input/third.mkv"), b"third").unwrap();
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
printf 'probe %s\n' "$last" >> execution-order
printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"matroska,webm","duration":"1"}}'"#,
    );

    let result = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();

    assert!(result.status.success(), "{:?}", result.stderr);
    let order = fs::read_to_string(fixture.0.join("execution-order")).unwrap();
    let phases: Vec<_> = order
        .lines()
        .map(|line| line.split_once(' ').unwrap().0)
        .collect();
    assert_eq!(
        phases,
        [
            "probe",
            "transcode",
            "probe",
            "transcode",
            "probe",
            "transcode"
        ],
        "{order}"
    );
}

#[test]
fn recursive_mode_processes_video_files_serially_and_preserves_the_directory_tree() {
    let fixture = Fixture::new(
        r#"previous=''
source=''
for argument do
    if test "$previous" = -i; then source=$argument; fi
    previous=$argument
done
mkdir running || exit 20
printf '%s\n' "$source" >> order
sleep 0.05
rmdir running
printf encoded > "$argument""#,
    );
    fs::create_dir_all(fixture.0.join("input/nested")).unwrap();
    fs::write(fixture.0.join("input/first"), b"first").unwrap();
    fs::write(fixture.0.join("input/nested/audio"), b"audio").unwrap();
    fs::write(fixture.0.join("input/nested/movie.m2ts"), b"movie").unwrap();
    fs::write(fixture.0.join("input/nested/notes.md"), b"notes").unwrap();
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
printf '%s\n' "$last" >> probe-order
case "$last" in
    input/nested/notes.md) printf NOT-MEDIA >&2; exit 1 ;;
    input/nested/audio) streams='[{"index":0,"codec_type":"audio"}]'; format=matroska,webm ;;
    input/nested/movie.m2ts) streams='[{"index":0,"codec_type":"video"}]'; format=mpegts ;;
    *) streams='[{"index":0,"codec_type":"video"}]'; format=matroska,webm ;;
esac
printf '{"streams":%s,"format":{"format_name":"%s","duration":"1"}}' "$streams" "$format""#,
    );

    let result = fixture
        .recursive_command("output")
        .args(["-C", "ts", "--copy"])
        .output()
        .unwrap();

    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(
        fs::read_to_string(fixture.0.join("order")).unwrap(),
        "input/first\ninput/nested/movie.m2ts\n"
    );
    assert_eq!(
        fs::read(fixture.0.join("output/first.ts")).unwrap(),
        b"encoded"
    );
    assert_eq!(
        fs::read(fixture.0.join("output/nested/movie.ts")).unwrap(),
        b"encoded"
    );
    assert!(!fixture.0.join("output/nested/audio").exists());
    assert!(!fixture.0.join("output/nested/notes.md").exists());
    assert!(!fixture.0.join("output/.part.first.ts").exists());
    assert!(!fixture.0.join("output/nested/.part.movie.ts").exists());
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains(
            "warning: skipping input/nested/notes.md: Skipped · Probe failed: probe failed"
        )
    );
    assert!(stderr.contains("NOT-MEDIA"));
    assert!(stdout.contains("batch summary: total 4 | succeeded 2 | failed 0 | skipped 2"));
}

#[test]
fn recursive_mode_continues_after_task_failures_and_exits_after_the_summary() {
    let fixture = Fixture::new(
        r#"previous=''
source=''
for argument do
    if test "$previous" = -i; then source=$argument; fi
    previous=$argument
done
printf '%s\n' "$source" >> started
case "$source" in
    input/bad.mkv) printf 'BAD-TRANSCODE\n' >&2; exit 9 ;;
esac
printf encoded > "$argument""#,
    );
    fs::create_dir(fixture.0.join("input")).unwrap();
    for input in ["first.mkv", "bad.mkv", "last.mkv"] {
        fs::write(fixture.0.join("input").join(input), b"video").unwrap();
    }

    let result = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        fs::read(fixture.0.join("output/first.mkv")).unwrap(),
        b"encoded"
    );
    assert_eq!(
        fs::read(fixture.0.join("output/last.mkv")).unwrap(),
        b"encoded"
    );
    assert!(!fixture.0.join("output/bad.mkv").exists());
    assert!(!fixture.0.join("output/.part.bad.mkv").exists());
    assert_eq!(
        fs::read_to_string(fixture.0.join("started"))
            .unwrap()
            .lines()
            .count(),
        3,
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("failed: input/bad.mkv: Failed · Transcoding: transcode failed"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("BAD-TRANSCODE").count(), 1, "{stderr}");
    assert!(
        stdout.ends_with(
            "batch summary: total 3 | succeeded 2 | failed 1 | skipped 0 | reused 0 | forced 0\n"
        ),
        "{stdout}"
    );
}
