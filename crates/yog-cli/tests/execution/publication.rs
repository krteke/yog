use super::support::Fixture;
use std::fs;

#[test]
fn only_the_part_file_is_written_and_success_is_published_by_rename() {
    let fixture = Fixture::new(
        r#"for last do :; done
case "$last" in .part.output.mkv|*/.part.output.mkv) ;; *) exit 8 ;; esac
test -f "$last" || exit 9
test ! -e output.mkv || exit 10
printf encoded > "$last"
printf 'frame=1\nout_time_us=1000000\nprogress=end\n'"#,
    );
    let result = fixture.command().arg("--copy").output().unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"encoded");
    assert!(!fixture.0.join(".part.output.mkv").exists());
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 4);
    assert!(String::from_utf8_lossy(&result.stdout).contains("complete:"));
    assert!(!result.stderr.contains(&0x1b));
}

#[track_caller]
fn assert_failed_transcode_preserves_target(body: &str) {
    let fixture = Fixture::new(body);
    fs::write(fixture.0.join("output.mkv"), b"original").unwrap();
    let result = fixture.command().args(["-O", "--copy"]).output().unwrap();
    assert_eq!(result.status.code(), Some(1), "{body}");
    assert_eq!(
        fs::read(fixture.0.join("output.mkv")).unwrap(),
        b"original",
        "{body}"
    );
    assert!(!fixture.0.join(".part.output.mkv").exists(), "{body}");
    assert!(
        !String::from_utf8_lossy(&result.stdout).contains("complete:"),
        "{body}"
    );
}

#[test]
fn failed_transcode_deletes_its_partial_output_and_preserves_the_target() {
    assert_failed_transcode_preserves_target(
        "for last do :; done; printf partial > \"$last\"; exit 7",
    );
}

#[test]
fn failed_transcode_after_progress_end_preserves_the_target() {
    assert_failed_transcode_preserves_target("printf 'progress=end\\n'; exit 7");
}

#[test]
fn empty_transcode_output_preserves_the_target() {
    assert_failed_transcode_preserves_target("exit 0");
}

#[test]
fn missing_transcode_output_preserves_the_target() {
    assert_failed_transcode_preserves_target("for last do :; done; rm \"$last\"; exit 0");
}

#[test]
fn overwrite_replaces_an_existing_target_after_success() {
    let fixture = Fixture::new("for last do :; done; printf replacement > \"$last\"");
    fs::write(fixture.0.join("output.mkv"), b"original").unwrap();
    let result = fixture.command().args(["-O", "--copy"]).output().unwrap();
    assert!(result.status.success());
    assert_eq!(
        fs::read(fixture.0.join("output.mkv")).unwrap(),
        b"replacement"
    );
}

#[test]
fn publication_preserves_a_target_created_by_a_racing_writer() {
    let fixture = Fixture::new(
        "for last do :; done; printf encoded > \"$last\"; printf racing-writer > output.mkv",
    );
    let result = fixture.command().arg("--copy").output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        fs::read(fixture.0.join("output.mkv")).unwrap(),
        b"racing-writer"
    );
    assert!(!fixture.0.join(".part.output.mkv").exists());
}

#[track_caller]
fn assert_existing_part_is_preserved(flags: &[&str]) {
    let fixture = Fixture::new("touch ffmpeg-started");
    fixture.tool("ffprobe", "touch ffprobe-started");
    fs::write(fixture.0.join(".part.output.mkv"), b"another attempt").unwrap();
    let result = fixture
        .command()
        .args(flags.iter().copied())
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1), "{flags:?}");
    assert_eq!(
        fs::read(fixture.0.join(".part.output.mkv")).unwrap(),
        b"another attempt",
        "{flags:?}"
    );
    assert!(!fixture.0.join("ffprobe-started").exists(), "{flags:?}");
    assert!(!fixture.0.join("ffmpeg-started").exists(), "{flags:?}");
}

#[test]
fn an_existing_part_is_never_overwritten_or_deleted() {
    assert_existing_part_is_preserved(&["--copy"]);
}

#[test]
fn overwrite_does_not_overwrite_or_delete_an_existing_part() {
    assert_existing_part_is_preserved(&["-O", "--copy"]);
}
