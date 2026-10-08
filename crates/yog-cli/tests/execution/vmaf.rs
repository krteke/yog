use super::support::Fixture;
use std::fs;

#[track_caller]
fn assert_published_vmaf(vmaf_args: &[&str], expected_mode: &str, subsampled: bool) {
    let fixture = Fixture::new(
        r#"printf '%s\n' "$*" >> ffmpeg-commands
filter=''
for argument do
    case "$argument" in *libvmaf=*) filter=$argument ;; esac
done
if test -n "$filter"; then
    test -f output.mkv || exit 20
    test ! -e .part.output.mkv || exit 21
    metrics=${filter#*log_path=\'}
    metrics=${metrics%%\':shortest=*}
    printf '%s' '{"pooled_metrics":{"vmaf":{"mean":95.25}}}' > "$metrics"
    exit 0
fi
for last do :; done
printf encoded > "$last""#,
    );
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
case "$last" in
    input.mkv)
        printf '%s' '{"streams":[{"index":2,"codec_type":"video","width":320,"height":180}],"format":{"format_name":"matroska,webm","duration":"1"}}'
        ;;
    output.mkv|*/output.mkv)
        printf '%s' '{"streams":[{"index":4,"codec_type":"video"}],"format":{"duration":"1"}}'
        ;;
    *) exit 22 ;;
esac"#,
    );

    let result = fixture
        .command()
        .args(vmaf_args.iter().copied())
        .arg("--copy")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"encoded");
    assert!(!fixture.0.join(".part.output.mkv").exists());
    assert!(
        stdout.contains(&format!(
            "vmaf: output.mkv | score 95.250 | {expected_mode}"
        )),
        "{stdout}"
    );
    assert!(stdout.find("complete:").unwrap() < stdout.find("vmaf:").unwrap());

    let commands = fs::read_to_string(fixture.0.join("ffmpeg-commands")).unwrap();
    let vmaf = commands
        .lines()
        .find(|command| command.contains("libvmaf="))
        .unwrap();
    assert!(vmaf.contains("[0:4]crop=w=320:h=180"), "{vmaf}");
    assert!(vmaf.contains("[1:2]setpts=PTS-STARTPTS"), "{vmaf}");
    assert!(vmaf.contains(":n_threads="), "{vmaf}");
    assert_eq!(vmaf.contains(":n_subsample=7"), subsampled, "{vmaf}");
}

#[test]
fn published_transcode_calculates_full_vmaf() {
    assert_published_vmaf(&["--vmaf"], "full", false);
}

#[test]
fn published_transcode_calculates_subsampled_vmaf() {
    assert_published_vmaf(&["--vmaf=7"], "n_subsample=7", true);
}

#[track_caller]
fn assert_requested_vmaf_failure(verbose: bool) {
    let fixture = Fixture::new(
        r#"case "$*" in
    *libvmaf=*) printf 'cannot score' >&2; exit 12 ;;
esac
for last do :; done
printf encoded > "$last""#,
    );
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
case "$last" in
    input.mkv)
        printf '%s' '{"streams":[{"index":0,"codec_type":"video","width":320,"height":180}],"format":{"format_name":"matroska,webm","duration":"1"}}'
        ;;
    *) printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"duration":"1"}}' ;;
esac"#,
    );

    fs::write(fixture.0.join("output.mkv"), b"previous").unwrap();
    let mut command = fixture.command();
    command.args(["--vmaf", "--copy", "-O"]);
    if verbose {
        command.arg("--verbose");
    }
    let result = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert!(stdout.contains("complete: output.mkv"), "{stdout}");
    assert!(
        stderr.contains("warning: vmaf: VMAF calculation failed"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("cannot score").count(), 1, "{stderr}");
    assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"encoded");
    assert!(!fixture.0.join(".part.output.mkv").exists());
}

#[test]
fn requested_vmaf_failure_does_not_fail_or_roll_back_the_transcode() {
    assert_requested_vmaf_failure(false);
}

#[test]
fn verbose_vmaf_failure_does_not_fail_or_roll_back_the_transcode() {
    assert_requested_vmaf_failure(true);
}
