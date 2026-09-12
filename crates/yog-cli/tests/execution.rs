#![cfg(unix)]

use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

// These are isolated test fixtures, not the application's output workflow.
struct Fixture(PathBuf);

impl Fixture {
    fn new(ffmpeg: &str) -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "yog-cli-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fs::write(fixture.0.join("input.mkv"), b"input").unwrap();
        fixture.tool(
            "ffprobe",
            "printf '%s' '{\"streams\":[{\"index\":0,\"codec_type\":\"video\"}],\"format\":{\"format_name\":\"matroska,webm\",\"duration\":\"1\"}}'",
        );
        fixture.tool("ffmpeg", ffmpeg);
        fixture
    }

    fn tool(&self, name: &str, body: &str) {
        let path = self.0.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yog"));
        command
            .current_dir(&self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .args([
                "input.mkv",
                "-o",
                "output.mkv",
                "--ffprobe",
                "./ffprobe",
                "--ffmpeg",
                "./ffmpeg",
                "--timeout",
                "3",
            ]);
        command
    }

    fn recursive_command(&self, output: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yog"));
        command
            .current_dir(&self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .args([
                "input",
                "-o",
                output,
                "--recursive",
                "--ffprobe",
                "./ffprobe",
                "--ffmpeg",
                "./ffmpeg",
            ]);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn only_the_part_file_is_written_and_success_is_published_by_rename() {
    let fixture = Fixture::new(
        r#"for last do :; done
case "$last" in output.mkv.part|*/output.mkv.part) ;; *) exit 8 ;; esac
test -f "$last" || exit 9
test ! -e output.mkv || exit 10
printf encoded > "$last"
printf 'frame=1\nout_time_us=1000000\nprogress=end\n'"#,
    );
    let result = fixture.command().arg("--copy").output().unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"encoded");
    assert!(!fixture.0.join("output.mkv.part").exists());
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 4);
    assert!(String::from_utf8_lossy(&result.stderr).contains("complete:"));
    assert!(!result.stderr.contains(&0x1b));
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
    assert!(!fixture.0.join("output/first.ts.part").exists());
    assert!(!fixture.0.join("output/nested/movie.ts.part").exists());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("warning: skipping input/nested/notes.md because ffprobe failed"));
    assert!(stderr.contains("NOT-MEDIA"));
}

#[test]
fn recursive_mode_preserves_implicit_suffixes_and_uses_their_exact_muxers() {
    let fixture = Fixture::new(
        r#"previous=''
muxer=''
for argument do
    if test "$previous" = -f; then muxer=$argument; fi
    previous=$argument
done
printf '%s %s\n' "$muxer" "$argument" >> commands
printf encoded > "$argument""#,
    );
    fs::create_dir(fixture.0.join("input")).unwrap();
    for extension in [
        "mkv", "webm", "mp4", "mov", "m4a", "3gp", "3g2", "f4v", "ismv", "psp", "ts", "m2ts",
        "avi", "flv", "asf", "wmv", "mpg", "mpeg", "vob", "ogg", "ogv",
    ] {
        fs::write(fixture.0.join(format!("input/video.{extension}")), b"video").unwrap();
    }
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
case "$last" in
    *.mkv|*.webm) format=matroska,webm ;;
    *.ts|*.m2ts) format=mpegts ;;
    *.avi) format=avi ;;
    *.flv) format=flv ;;
    *.asf|*.wmv) format=asf ;;
    *.mpg|*.mpeg|*.vob) format=mpeg ;;
    *.ogg|*.ogv) format=ogg ;;
    *) format=mov,mp4,m4a,3gp,3g2,mj2 ;;
esac
printf '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"%s","duration":"1"}}' "$format""#,
    );

    let result = fixture
        .recursive_command("output")
        .arg("--copy")
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);

    for extension in [
        "mkv", "webm", "mp4", "mov", "m4a", "3gp", "3g2", "f4v", "ismv", "psp", "ts", "m2ts",
        "avi", "flv", "asf", "wmv", "mpg", "mpeg", "vob", "ogg", "ogv",
    ] {
        assert_eq!(
            fs::read(fixture.0.join(format!("output/video.{extension}"))).unwrap(),
            b"encoded",
            "{extension}"
        );
    }
    let commands = fs::read_to_string(fixture.0.join("commands")).unwrap();
    for (muxer, extension) in [
        ("matroska", "mkv"),
        ("webm", "webm"),
        ("mp4", "mp4"),
        ("mov", "mov"),
        ("ipod", "m4a"),
        ("3gp", "3gp"),
        ("3g2", "3g2"),
        ("f4v", "f4v"),
        ("ismv", "ismv"),
        ("psp", "psp"),
        ("mpegts", "ts"),
        ("mpegts", "m2ts"),
        ("avi", "avi"),
        ("flv", "flv"),
        ("asf", "asf"),
        ("asf", "wmv"),
        ("mpeg", "mpg"),
        ("mpeg", "mpeg"),
        ("vob", "vob"),
        ("ogg", "ogg"),
        ("ogv", "ogv"),
    ] {
        assert!(
            commands
                .lines()
                .any(|line| line == format!("{muxer} output/video.{extension}.part")),
            "missing {muxer} mapping for {extension}:\n{commands}"
        );
    }
}

#[test]
fn recursive_mode_uses_core_container_resolution_before_starting_ffmpeg() {
    let fixture = Fixture::new(
        r#"touch ffmpeg-started
for last do :; done
printf encoded > "$last""#,
    );
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/clip.nut"), b"video").unwrap();
    fixture.tool(
        "ffprobe",
        r#"printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"nut","duration":"1"}}'"#,
    );

    let result = fixture
        .recursive_command("unsupported")
        .arg("--copy")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("cannot use input format Some(\"nut\") as an output container")
    );
    assert!(!fixture.0.join("ffmpeg-started").exists());

    let result = fixture
        .recursive_command("converted")
        .args(["-C", "mkv", "--copy"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(
        fs::read(fixture.0.join("converted/clip.mkv")).unwrap(),
        b"encoded"
    );
}

#[test]
fn recursive_mode_rejects_output_collisions_before_starting_ffmpeg() {
    let fixture = Fixture::new("touch ffmpeg-started");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/same.mkv"), b"first").unwrap();
    fs::write(fixture.0.join("input/same.avi"), b"second").unwrap();
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
case "$last" in *.mkv) format=matroska,webm ;; *) format=avi ;; esac
printf '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"%s"}}' "$format""#,
    );

    let result = fixture
        .recursive_command("output")
        .args(["-C", "webm", "-O", "--copy"])
        .output()
        .unwrap();

    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("output/same.webm"));
    assert!(!fixture.0.join("ffmpeg-started").exists());
}

#[test]
fn temporary_covers_are_readable_and_cleaned_on_success_or_failure() {
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
    for failure in [
        "none",
        "empty",
        "extraction",
        "second",
        "encoding",
        "extraction-timeout",
        "encoding-timeout",
        "tempdir",
    ] {
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
        assert!(!fixture.0.join("output.mkv.part").exists(), "{failure}");
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
            assert_eq!(paths[0].parent(), paths[1].parent());
            assert_ne!(paths[0], paths[1]);
            assert!(
                paths
                    .iter()
                    .all(|path| path.starts_with(&temporary) && !path.exists())
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
}

#[test]
fn verification_is_opt_in_and_warnings_do_not_prevent_publication() {
    let fixture = Fixture::new("for last do :; done; printf encoded > \"$last\"");
    fixture.tool("ffprobe", r#"
printf '%s\n' "$*" >> probe-calls
for last do :; done
case "$last" in
  input.mkv) printf '%s' '{"streams":[{"index":8,"codec_type":"audio","codec_name":"aac"}],"format":{"format_name":"matroska,webm","tags":{"title":"original"}}}' ;;
  *.part)
    test ! -e output.mkv || exit 8
    test "$(cat "$last")" = encoded || exit 9
    printf '%s' '{"streams":[],"format":{"tags":{"title":"changed"}}}' ;;
  *) exit 10 ;;
esac
"#);
    let result = fixture.command().arg("--copy").output().unwrap();
    assert!(result.status.success());
    let calls = fs::read_to_string(fixture.0.join("probe-calls")).unwrap();
    assert_eq!(calls.lines().count(), 1);
    assert!(!calls.contains("-show_data_hash"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("warning: verify:"));
    for flags in [["-v", "--copy"], ["--copy", "--verify"]] {
        fs::remove_file(fixture.0.join("output.mkv")).unwrap();
        fs::write(fixture.0.join("probe-calls"), "").unwrap();
        let result = fixture.command().args(flags).output().unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{stderr}");
        assert!(stderr.contains("audio stream #8: missing"), "{stderr}");
        assert!(stderr.contains("metadata \"title\""), "{stderr}");
        assert!(stderr.find("warning: verify:").unwrap() < stderr.find("complete:").unwrap());
        assert!(!fixture.0.join("output.mkv.part").exists());
        assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"encoded");
        let calls = fs::read_to_string(fixture.0.join("probe-calls")).unwrap();
        assert_eq!(calls.lines().count(), 2);
        assert!(
            calls
                .lines()
                .all(|line| line.contains("-show_data_hash sha256"))
        );
    }
}

#[test]
fn verification_uses_planned_cover_tags_and_preserves_content_checks() {
    use serde_json::{Value, json};
    let fixture = Fixture::new(
        r#"
case "$*" in *image2pipe*) printf image; exit 0 ;; esac
for last do :; done
printf encoded > "$last"
"#,
    );
    fixture.tool(
        "ffprobe",
        r#"
for last do :; done
case "$last" in input.mkv) prefix=input ;; *) prefix=output ;; esac
case "$*" in *-show_packets*) cat "$prefix-packets.json" ;; *) cat "$prefix.json" ;; esac
"#,
    );
    let mut original = json!({"streams":[
        {"index":8,"codec_type":"video","codec_name":"png","pix_fmt":"yuv420p","disposition":{"attached_pic":1}},
        {"index":2,"codec_type":"audio","codec_name":"aac"},
        {"index":5,"codec_type":"video","codec_name":"mjpeg","pix_fmt":"yuv420p","disposition":{"attached_pic":1},"tags":{"filename":"back.jpg","mimetype":"image/jpeg","title":"Keep this"}}
    ],"format":{"format_name":"matroska,webm"}});
    original["pixel_formats"] = serde_json::from_str::<Value>(include_str!(
        "../../yog-core/src/ffmpeg/test_pixel_formats.json"
    ))
    .unwrap()["pixel_formats"]
        .clone();
    fs::write(fixture.0.join("input.json"), original.to_string()).unwrap();
    let source_packets = json!({"packets":[
        {"stream_index":8,"data_hash":"SHA256:front","pts_time":"0.500000","duration_time":"10.000000"},
        {"stream_index":2,"data_hash":"SHA256:audio","pts_time":"0.000000","duration_time":"0.100000"},
        {"stream_index":5,"data_hash":"SHA256:back","pts_time":"0.000000"}
    ]});
    fs::write(
        fixture.0.join("input-packets.json"),
        source_packets.to_string(),
    )
    .unwrap();

    for (case, expected) in [
        ("ok", None),
        ("missing filename", Some("metadata \"filename\"")),
        ("wrong mime", Some("metadata \"mimetype\"")),
        ("changed original title", Some("metadata \"title\"")),
        ("unplanned tag", Some("metadata \"extra\"")),
        ("changed image", Some("cover stream #8 -> #1: packet data")),
        ("lost cover", Some("cover stream #5: missing")),
        ("cover as video", Some("video stream #2: added")),
        (
            "audio timing",
            Some("audio stream #2 -> #0: packet presentation"),
        ),
        ("unplanned mp4 tags", Some("metadata \"FILENAME\": added")),
    ] {
        let mut actual = original.clone();
        actual["streams"] = json!([
            original["streams"][1],
            original["streams"][0],
            original["streams"][2]
        ]);
        for (index, stream) in actual["streams"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            stream["index"] = json!(index);
        }
        actual["streams"][1]["tags"] = json!({"FILENAME":"cover.png","MIMETYPE":"image/png"});
        let mut packets = source_packets.clone();
        packets["packets"][0]["stream_index"] = json!(1);
        packets["packets"][0]["pts_time"] = json!("0.000000");
        packets["packets"][0]["duration_time"] = Value::Null;
        packets["packets"][1]["stream_index"] = json!(0);
        packets["packets"][2]["stream_index"] = json!(2);
        match case {
            "ok" | "unplanned mp4 tags" => {}
            "missing filename" => {
                actual["streams"][1]["tags"]
                    .as_object_mut()
                    .unwrap()
                    .remove("FILENAME");
            }
            "wrong mime" => actual["streams"][1]["tags"]["MIMETYPE"] = json!("image/jpeg"),
            "changed original title" => actual["streams"][2]["tags"]["title"] = json!("Lost"),
            "unplanned tag" => actual["streams"][1]["tags"]["extra"] = json!("unexpected"),
            "changed image" => packets["packets"][0]["data_hash"] = json!("SHA256:changed"),
            "lost cover" => {
                actual["streams"].as_array_mut().unwrap().pop();
            }
            "cover as video" => actual["streams"][2]["disposition"]["attached_pic"] = json!(0),
            "audio timing" => packets["packets"][1]["pts_time"] = json!("0.200000"),
            _ => unreachable!(),
        }
        fs::write(fixture.0.join("output.json"), actual.to_string()).unwrap();
        fs::write(fixture.0.join("output-packets.json"), packets.to_string()).unwrap();
        let mut command = fixture.command();
        command.args(["--copy", "--verify", "-O"]);
        if case == "unplanned mp4 tags" {
            command.args(["-C", "mp4"]);
        }
        let result = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{case}: {stderr}");
        match expected {
            Some(expected) => assert!(stderr.contains(expected), "{case}: {stderr}"),
            None => assert!(!stderr.contains("warning: verify:"), "{stderr}"),
        }
        assert_eq!(
            stderr
                .lines()
                .any(|line| line.contains("cover stream") && line.contains("packet presentation")),
            case == "unplanned mp4 tags",
            "{case}: {stderr}"
        );
        assert!(!fixture.0.join("output.mkv.part").exists());
    }
}

#[test]
fn verification_probe_and_packet_failures_warn_instead_of_deleting_the_output() {
    let fixture = Fixture::new("for last do :; done; printf encoded > \"$last\"");
    for failure in ["metadata", "packets"] {
        fixture.tool(
            "ffprobe",
            &format!(
                r#"
for last do :; done
case "$*" in *-show_packets*) echo 'broken packet probe' >&2; exit 7 ;; esac
if test "$last" != input.mkv && test '{failure}' = metadata; then
  echo 'broken output probe' >&2
  exit 6
fi
printf '%s' '{{"streams":[{{"index":0,"codec_type":"audio"}}],"format":{{"format_name":"matroska,webm"}}}}'
"#
            ),
        );
        let result = fixture
            .command()
            .args(["--copy", "--verify", "-O"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{stderr}");
        assert!(stderr.contains("verification incomplete"), "{stderr}");
        assert!(stderr.contains("broken"), "{stderr}");
        assert!(stderr.contains("complete:"));
        assert!(!fixture.0.join("output.mkv.part").exists());
        assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"encoded");
    }
}

#[test]
fn missing_audio_after_seek_fails_verification_and_keeps_the_previous_output() {
    use serde_json::{Value, json};

    let fixture = Fixture::new(
        r#"
case "$*" in
    *'-h encoder=libx264'*)
        printf 'Encoder libx264 [H264]:\n Supported pixel formats: yuv420p\n'
        exit 0
        ;;
    *framehash*)
        printf '#format: frame checksums\n0, 0, 0, 1, 1, hash\n'
        case "$*" in *output.mkv.part*) ;; *) printf '1, 0, 0, 1, 1, hash\n' ;; esac
        exit 0
        ;;
esac
for last do :; done
printf encoded > "$last"
"#,
    );
    fixture.tool(
        "ffprobe",
        r#"
for last do :; done
case "$last" in input.mkv) cat input.json ;; *) cat output.json ;; esac
"#,
    );
    let mut media = json!({
        "streams": [
            {"index":0,"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p"},
            {"index":1,"codec_type":"audio","codec_name":"aac"}
        ],
        "format":{"format_name":"matroska,webm","duration":"20.000000"}
    });
    media["pixel_formats"] = serde_json::from_str::<Value>(include_str!(
        "../../yog-core/src/ffmpeg/test_pixel_formats.json"
    ))
    .unwrap()["pixel_formats"]
        .clone();
    fs::write(fixture.0.join("input.json"), media.to_string()).unwrap();
    fs::write(fixture.0.join("output.json"), media.to_string()).unwrap();
    fs::write(fixture.0.join("output.mkv"), b"previous").unwrap();

    let result = fixture
        .command()
        .args(["--encode-x264", "--verify", "-O"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(
            "verification failed: audio stream #1 -> #1 has no decoded frames after seeking to 5.000000s"
        ),
        "{stderr}"
    );
    assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"previous");
    assert!(!fixture.0.join("output.mkv.part").exists());
}

#[test]
fn failures_delete_only_our_part_and_preserve_existing_targets() {
    let fixture = Fixture::new("");
    for body in [
        "for last do :; done; printf partial > \"$last\"; exit 7",
        "printf 'progress=end\\n'; exit 7",
        "exit 0", // The exclusively created part is still empty.
        "for last do :; done; rm \"$last\"; exit 0",
    ] {
        fixture.tool("ffmpeg", body);
        fs::write(fixture.0.join("output.mkv"), b"original").unwrap();
        let result = fixture.command().args(["-O", "--copy"]).output().unwrap();
        assert_eq!(result.status.code(), Some(1), "{body}");
        assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"original");
        assert!(!fixture.0.join("output.mkv.part").exists());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("complete:"));
    }
    fixture.tool(
        "ffmpeg",
        "for last do :; done; printf replacement > \"$last\"",
    );
    let result = fixture.command().args(["-O", "--copy"]).output().unwrap();
    assert!(result.status.success());
    assert_eq!(
        fs::read(fixture.0.join("output.mkv")).unwrap(),
        b"replacement"
    );

    fs::remove_file(fixture.0.join("output.mkv")).unwrap();
    fixture.tool(
        "ffmpeg",
        "for last do :; done; printf encoded > \"$last\"; printf racing-writer > output.mkv",
    );
    let result = fixture.command().arg("--copy").output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        fs::read(fixture.0.join("output.mkv")).unwrap(),
        b"racing-writer"
    );
    assert!(!fixture.0.join("output.mkv.part").exists());
}

#[test]
fn an_existing_part_is_never_overwritten_or_deleted_even_with_overwrite() {
    let fixture = Fixture::new("touch ffmpeg-started");
    fixture.tool("ffprobe", "touch ffprobe-started");
    fs::write(fixture.0.join("output.mkv.part"), b"another attempt").unwrap();
    for flags in [vec!["--copy"], vec!["-O", "--copy"]] {
        let result = fixture.command().args(flags).output().unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert_eq!(
            fs::read(fixture.0.join("output.mkv.part")).unwrap(),
            b"another attempt"
        );
        assert!(!fixture.0.join("ffprobe-started").exists());
        assert!(!fixture.0.join("ffmpeg-started").exists());
    }
}

#[test]
fn probe_and_ffmpeg_diagnostics_are_emitted_once_in_both_output_modes() {
    let fixture =
        Fixture::new("printf 'FFMPEG-DIAGNOSTIC\\n' >&2; printf 'progress=end\\n'; exit 7");
    for verbose in [false, true] {
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
    // Encoder-help errors are nested in PlanError; retain their diagnostics.
    let result = fixture.command().arg("--encode-x264").output().unwrap();
    let error = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(error.matches("FFMPEG-DIAGNOSTIC").count(), 1, "{error}");
    assert_eq!(error.matches("process failed").count(), 1, "{error}");
    assert!(!fixture.0.join("output.mkv.part").exists());
    fixture.tool(
        "ffprobe",
        "printf 'FFPROBE-DIAGNOSTIC\\n' >&2; printf '{}'; exit 8",
    );
    for verbose in [false, true] {
        let mut command = fixture.command();
        if verbose {
            command.arg("--verbose");
        }
        let result = command.arg("--copy").output().unwrap();
        let error = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.code(), Some(1));
        assert_eq!(error.matches("FFPROBE-DIAGNOSTIC").count(), 1, "{error}");
        assert!(error.contains("probe failed"));
        assert!(!fixture.0.join("output.mkv.part").exists());
    }
}

// Always reap a failed test's CLI process instead of leaving it in the background.
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.0.id())])
            .stderr(Stdio::null())
            .status();
        let _ = self.0.wait();
    }
}

#[test]
fn timeout_reaps_child_and_cleans_part_even_when_stderr_is_not_consumed() {
    let fixture = Fixture::new(
        r#"printf '%s' "$$" > child-pid
for last do :; done
printf partial > "$last"
i=0; while [ "$i" -lt 10000 ]; do printf 'diagnostic-data\n' >&2; i=$((i+1)); done
while :; do :; done"#,
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_yog"));
    let mut child = Running(
        command
            .current_dir(&fixture.0)
            .env("XDG_CONFIG_HOME", &fixture.0)
            .args([
                "input.mkv",
                "-o",
                "output.mkv",
                "--ffprobe",
                "./ffprobe",
                "--ffmpeg",
                "./ffmpeg",
                "--timeout",
                "1",
                "--verbose",
                "--copy",
            ])
            .process_group(0)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "CLI blocked on stderr after timeout"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(1));
    assert!(!fixture.0.join("output.mkv.part").exists());
    assert!(!fixture.0.join("output.mkv").exists());
    let pid = fs::read_to_string(fixture.0.join("child-pid")).unwrap();
    assert!(
        !PathBuf::from(format!("/proc/{pid}")).exists(),
        "FFmpeg was not reaped"
    );
}

#[test]
fn cancellation_during_probe_encoding_or_verification_cleans_only_the_part() {
    for phase in [
        "ffprobe",
        "ffmpeg",
        "encoder-help",
        "cover-extraction",
        "cover-encoding",
        "verify",
        "verify-packets",
    ] {
        let fixture = Fixture::new("");
        let temporary = fixture.0.join("temporary covers");
        fs::create_dir(&temporary).unwrap();
        let blocking = "printf '%s' \"$$\" > child-pid; printf waiting >&2; while :; do :; done";
        if phase.starts_with("cover-") {
            fixture.tool("ffprobe", r#"printf '%s' '{"streams":[{"index":0,"codec_type":"audio"},{"index":1,"codec_type":"video","codec_name":"png","disposition":{"attached_pic":1}}],"format":{"format_name":"matroska,webm"}}'"#);
            let extract = if phase == "cover-extraction" {
                blocking
            } else {
                "printf image; exit 0"
            };
            fixture.tool(
                "ffmpeg",
                &format!("case \"$*\" in *image2pipe*) {extract} ;; esac\n{blocking}"),
            );
        } else if phase.starts_with("verify") {
            fixture.tool("ffmpeg", "for last do :; done; printf encoded > \"$last\"");
            let condition = if phase == "verify" {
                "for last do :; done; test \"$last\" != input.mkv"
            } else {
                "case \"$*\" in *-show_packets*) true ;; *) false ;; esac"
            };
            fixture.tool("ffprobe", &format!("if {condition}; then {blocking}; fi\nprintf '%s' '{{\"streams\":[{{\"index\":0,\"codec_type\":\"audio\"}}],\"format\":{{\"format_name\":\"matroska,webm\"}}}}'"));
        } else {
            fixture.tool(
                if phase == "encoder-help" {
                    "ffmpeg"
                } else {
                    phase
                },
                blocking,
            );
        }
        fs::write(fixture.0.join("output.mkv"), b"original").unwrap();
        let mut child = Running(
            fixture
                .command()
                .env("TMPDIR", &temporary)
                .args([
                    "-O",
                    "--verify",
                    if phase == "encoder-help" {
                        "--encode-x264"
                    } else {
                        "--copy"
                    },
                ])
                .process_group(0)
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let started = Instant::now();
        while !fixture.0.join("child-pid").exists() {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "{phase} did not start"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            Command::new("kill")
                .args(["-INT", &child.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "cancellation did not complete"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(130), "{phase}");
        assert_eq!(
            fs::read_dir(&temporary).unwrap().count(),
            0,
            "{phase}: leaked cover directory"
        );
        assert!(!fixture.0.join("output.mkv.part").exists());
        assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"original");
        let pid = fs::read_to_string(fixture.0.join("child-pid")).unwrap();
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    }
}

#[test]
fn ctrl_c_can_interrupt_log_drain_after_the_output_is_committed() {
    let fixture = Fixture::new(
        r#"for last do :; done
printf encoded > "$last"
i=0; while [ "$i" -lt 10000 ]; do printf 'diagnostic-data\n' >&2; i=$((i+1)); done"#,
    );
    let mut child = Running(
        fixture
            .command()
            .args(["--verbose", "--copy"])
            .process_group(0)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let started = Instant::now();
    while !fixture.0.join("output.mkv").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "output was not committed"
        );
        thread::sleep(Duration::from_millis(10));
    }
    // The child process has succeeded. Only an undrained log queue keeps the CLI
    // alive; blocking the runtime while joining it would disable Ctrl+C here.
    assert!(child.0.try_wait().unwrap().is_none());
    assert!(
        Command::new("kill")
            .args(["-INT", &child.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "signal handling blocked during log drain"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(status.code(), Some(0));
    assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"encoded");
    assert!(!fixture.0.join("output.mkv.part").exists());
}
