use super::support::Fixture;
use std::{fs, path::PathBuf};

#[track_caller]
fn assert_cover_cleanup(failure: &str) {
    let fixture = Fixture::new(
        r#"
case "$*" in
    *image2pipe*)
        case "$COVER_FAILURE" in
            empty) exit 0 ;;
            extraction) printf partial; printf EXTRACTION-ERROR >&2; exit 7 ;;
            second) case "$*" in *'0:3'*) printf partial; exit 8 ;; esac ;;
            extraction-timeout) printf partial; while :; do :; done ;;
        esac
        printf original-image
        exit 0 ;;
esac
previous=''
for arg do
    if test "$previous" = -attach; then
        test "$(cat "$arg")" = original-image || exit 9
        printf '%s\n' "$arg" >> cover-paths
    fi
    previous="$arg"
done
printf partial > "$arg"
case "$COVER_FAILURE" in
    encoding) printf ENCODING-ERROR >&2; exit 10 ;;
    encoding-timeout) while :; do :; done ;;
esac
printf encoded > "$arg"
"#,
    );
    fixture.tool("ffprobe", r#"printf '%s' '{"streams":[{"index":1,"codec_type":"video","codec_name":"png","disposition":{"attached_pic":1}},{"index":2,"codec_type":"audio"},{"index":3,"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}}],"format":{"format_name":"matroska,webm"}}'"#);
    let temporary = fixture.0.join("temporary covers");
    fs::create_dir(&temporary).unwrap();
    fs::write(fixture.0.join("output.mkv"), b"original").unwrap();
    fs::write(fixture.0.join("cover-paths"), b"").unwrap();
    let result = fixture
        .command()
        .env(
            "TMPDIR",
            if failure == "tempdir" {
                temporary.join("missing")
            } else {
                temporary.clone()
            },
        )
        .env("COVER_FAILURE", failure)
        .args(["--copy", "-O", "--timeout", "1"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert_eq!(
        result.status.code(),
        Some(if failure == "none" { 0 } else { 1 }),
        "{failure}: {stderr}"
    );
    let expected: &[u8] = if failure == "none" {
        b"encoded"
    } else {
        b"original"
    };
    assert_eq!(
        fs::read(fixture.0.join("output.mkv")).unwrap(),
        expected,
        "{failure}"
    );
    assert!(!fixture.0.join(".part.output.mkv").exists(), "{failure}");
    assert_eq!(
        fs::read_dir(&temporary).unwrap().count(),
        0,
        "{failure}: leaked cover directory"
    );
    let paths: Vec<_> = fs::read_to_string(fixture.0.join("cover-paths"))
        .unwrap()
        .lines()
        .map(PathBuf::from)
        .collect();
    if matches!(failure, "none" | "encoding" | "encoding-timeout") {
        assert_eq!(paths.len(), 2, "{failure}");
        assert_eq!(paths[0].parent(), paths[1].parent(), "{failure}");
        assert_ne!(paths[0], paths[1], "{failure}");
        assert!(
            paths
                .iter()
                .all(|path| path.starts_with(&temporary) && !path.exists()),
            "{failure}: {paths:?}"
        );
    } else {
        assert!(
            paths.is_empty(),
            "{failure}: encoding started after extraction failed"
        );
    }
    if failure == "empty" {
        assert!(
            stderr.contains("cover extraction produced no image data"),
            "{stderr}"
        );
    }
}

#[test]
fn temporary_covers_are_readable_and_cleaned_on_success() {
    assert_cover_cleanup("none");
}

#[test]
fn empty_cover_extraction_preserves_the_target_and_cleans_temporary_files() {
    assert_cover_cleanup("empty");
}

#[test]
fn failed_cover_extraction_preserves_the_target_and_cleans_temporary_files() {
    assert_cover_cleanup("extraction");
}

#[test]
fn failed_second_cover_extraction_cleans_the_first_cover() {
    assert_cover_cleanup("second");
}

#[test]
fn failed_cover_encoding_preserves_the_target_and_cleans_temporary_files() {
    assert_cover_cleanup("encoding");
}

#[test]
fn cover_extraction_timeout_preserves_the_target_and_cleans_temporary_files() {
    assert_cover_cleanup("extraction-timeout");
}

#[test]
fn cover_encoding_timeout_preserves_the_target_and_cleans_temporary_files() {
    assert_cover_cleanup("encoding-timeout");
}

#[test]
fn missing_cover_tempdir_preserves_the_target() {
    assert_cover_cleanup("tempdir");
}
