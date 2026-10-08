use super::support::Fixture;
use std::fs;

#[track_caller]
fn assert_ffmpeg_diagnostic(verbose: bool) {
    let fixture =
        Fixture::new("printf 'FFMPEG-DIAGNOSTIC\\n' >&2; printf 'progress=end\\n'; exit 7");
    let mut command = fixture.command();
    if verbose {
        command.arg("--verbose");
    }
    let result = command.arg("--copy").output().unwrap();
    let error = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(error.matches("FFMPEG-DIAGNOSTIC").count(), 1, "{error}");
    assert!(error.contains("transcode failed"));
    assert!(!error.contains("complete:"));
}

#[test]
fn ffmpeg_diagnostic_is_emitted_once() {
    assert_ffmpeg_diagnostic(false);
}

#[test]
fn verbose_ffmpeg_diagnostic_is_emitted_once() {
    assert_ffmpeg_diagnostic(true);
}

#[test]
fn encoder_help_diagnostic_is_emitted_once() {
    let fixture =
        Fixture::new("printf 'FFMPEG-DIAGNOSTIC\\n' >&2; printf 'progress=end\\n'; exit 7");
    // Encoder-help errors are nested in PlanError; retain their diagnostics.
    let result = fixture.command().arg("--encode-x264").output().unwrap();
    let error = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(error.matches("FFMPEG-DIAGNOSTIC").count(), 1, "{error}");
    assert_eq!(error.matches("process failed").count(), 1, "{error}");
    assert!(!fixture.0.join(".part.output.mkv").exists());
}

#[track_caller]
fn assert_ffprobe_diagnostic(verbose: bool) {
    let fixture = Fixture::new("touch ffmpeg-started");
    fixture.tool(
        "ffprobe",
        "printf 'FFPROBE-DIAGNOSTIC\\n' >&2; printf '{}'; exit 8",
    );
    let mut command = fixture.command();
    if verbose {
        command.arg("--verbose");
    }
    let result = command.arg("--copy").output().unwrap();
    let error = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(error.matches("FFPROBE-DIAGNOSTIC").count(), 1, "{error}");
    assert!(error.contains("probe failed"));
    assert!(!fixture.0.join(".part.output.mkv").exists());
}

#[test]
fn ffprobe_diagnostic_is_emitted_once() {
    assert_ffprobe_diagnostic(false);
}

#[test]
fn verbose_ffprobe_diagnostic_is_emitted_once() {
    assert_ffprobe_diagnostic(true);
}

#[test]
fn prediction_diagnostic_is_emitted_once_with_the_requested_quality_parameter() {
    let fixture = Fixture::new("");
    fixture.tool(
        "ffprobe",
        r#"printf '%s' '{"streams":[{"index":0,"codec_type":"video","width":320,"height":180,"pix_fmt":"yuv420p"}],"format":{"format_name":"matroska,webm","duration":"1"},"pixel_formats":[{"name":"yuv420p","nb_components":3,"log2_chroma_w":1,"log2_chroma_h":1,"flags":{"rgb":0,"alpha":0,"palette":0,"hwaccel":0},"components":[{"bit_depth":8},{"bit_depth":8},{"bit_depth":8}]}]}'"#,
    );
    fixture.tool(
        "ffmpeg",
        "printf '%s' \"$*\" > ffmpeg-args; printf 'PREDICTION-DIAGNOSTIC\\n' >&2; exit 22",
    );
    let result = fixture
        .analysis_command()
        .args(["predict", "--encode-vaapi", "av1", "--quality", "28"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(error.matches("PREDICTION-DIAGNOSTIC").count(), 1, "{error}");
    assert_eq!(error.matches("process failed").count(), 1, "{error}");
    assert!(error.contains("prediction failed"));
    let args = fs::read_to_string(fixture.0.join("ffmpeg-args")).unwrap();
    assert!(args.contains("-global_quality:v 28"), "{args}");
    assert!(!args.contains("-qp:v"), "{args}");
}
