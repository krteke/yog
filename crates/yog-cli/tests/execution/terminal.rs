use super::support::Fixture;

#[test]
fn quiet_disables_runtime_terminal_output_on_success() {
    assert_quiet_output(
        "for last do :; done; printf diagnostic >&2; printf encoded > \"$last\"",
        0,
        true,
    );
}

#[test]
fn quiet_disables_runtime_terminal_output_on_failure() {
    assert_quiet_output("printf diagnostic >&2; exit 9", 1, false);
}

#[track_caller]
fn assert_quiet_output(ffmpeg: &str, expected_status: i32, output_exists: bool) {
    let fixture = Fixture::new(ffmpeg);
    let result = fixture
        .command()
        .env("RUST_LOG", "debug")
        .args(["--quiet", "--copy"])
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(expected_status));
    assert!(result.stdout.is_empty(), "{:?}", result.stdout);
    assert!(result.stderr.is_empty(), "{:?}", result.stderr);
    assert_eq!(fixture.0.join("output.mkv").exists(), output_exists);
    assert!(!fixture.0.join(".part.output.mkv").exists());
}
