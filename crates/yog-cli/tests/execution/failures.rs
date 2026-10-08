use super::support::{Fixture, read_report};
use std::fs;

#[test]
fn invalid_encoder_quality_is_rejected_before_external_tools_start() {
    let fixture = Fixture::new("touch ffmpeg-started");
    fixture.tool("ffprobe", "touch ffprobe-started");

    let result = fixture
        .command()
        .args(["--encode-svt-av1", "--quality", "64"])
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&result.stderr),
        "libsvtav1 quality must be in 0..=63, got 64\n"
    );
    assert!(!fixture.0.join("ffprobe-started").exists());
    assert!(!fixture.0.join("ffmpeg-started").exists());
}

#[test]
fn output_preparation_failure_is_classified_before_probing() {
    let fixture = Fixture::new("touch ffmpeg-started");
    fixture.tool("ffprobe", "touch ffprobe-started");
    fs::write(fixture.0.join(".part.output.mkv"), b"existing attempt").unwrap();
    let result = fixture
        .command()
        .args(["--copy", "--report", "report.jsonl"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(!fixture.0.join("ffprobe-started").exists());
    assert_eq!(
        read_report(&fixture, "report.jsonl")[0]["transcode"]["result"]["phase"],
        "preparing_output"
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("Failed · Output preparation"));
}

#[test]
fn single_input_probe_failure_is_failed_not_skipped() {
    let fixture = Fixture::new("touch ffmpeg-started");
    fixture.tool("ffprobe", "exit 7");
    let result = fixture
        .command()
        .args(["--copy", "--report", "report.jsonl"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(!fixture.0.join("ffmpeg-started").exists());
    let rows = read_report(&fixture, "report.jsonl");
    assert_eq!(rows[0]["transcode"]["result"]["status"], "failed");
    assert_eq!(rows[0]["transcode"]["result"]["phase"], "probing");
    assert!(String::from_utf8_lossy(&result.stderr).contains("Failed · Probing"));
}

#[test]
fn encoder_capability_failure_is_classified_as_planning() {
    let fixture = Fixture::new("exit 7");
    let result = fixture
        .command()
        .args(["--encode-x264", "--report", "report.jsonl"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        read_report(&fixture, "report.jsonl")[0]["transcode"]["result"]["phase"],
        "planning"
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("Failed · Planning"));
}

#[test]
fn publication_failure_is_not_classified_as_transcoding() {
    let fixture =
        Fixture::new("for last do :; done; printf encoded > \"$last\"; printf other > output.mkv");
    let result = fixture
        .command()
        .args(["--copy", "--report", "report.jsonl"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"other");
    assert_eq!(
        read_report(&fixture, "report.jsonl")[0]["transcode"]["result"]["phase"],
        "publishing"
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("Failed · Publishing"));
}
