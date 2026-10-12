use super::support::{Fixture, read_report};
use std::fs;

fn verification_fixture() -> Fixture {
    let fixture = Fixture::new("for last do :; done; printf encoded > \"$last\"");
    fixture.tool("ffprobe", r#"
printf '%s\n' "$*" >> probe-calls
for last do :; done
case "$last" in
  input.mkv) printf '%s' '{"streams":[{"index":8,"codec_type":"audio","codec_name":"aac"}],"format":{"format_name":"matroska,webm","tags":{"title":"original"}}}' ;;
  .part.*|*/.part.*)
    test ! -e output.mkv || exit 8
    test "$(cat "$last")" = encoded || exit 9
    printf '%s' '{"streams":[],"format":{"tags":{"title":"changed"}}}' ;;
  *) exit 10 ;;
esac
"#);
    fixture
}

#[test]
fn verification_is_opt_in() {
    let fixture = verification_fixture();
    let result = fixture.command().arg("--copy").output().unwrap();
    assert!(result.status.success());
    let calls = fs::read_to_string(fixture.0.join("probe-calls")).unwrap();
    assert_eq!(calls.lines().count(), 1);
    assert!(!calls.contains("-show_data_hash"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("warning: verify:"));
}

#[track_caller]
fn assert_verification_warning_is_published(flags: [&str; 2]) {
    let fixture = verification_fixture();
    let result = fixture.command().args(flags).output().unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{flags:?}: {stderr}");
    assert!(
        stderr.contains("audio stream #8: missing"),
        "{flags:?}: {stderr}"
    );
    assert!(stderr.contains("metadata \"title\""), "{flags:?}: {stderr}");
    assert!(
        stdout.contains("complete: output.mkv"),
        "{flags:?}: {stdout}"
    );
    assert!(!fixture.0.join(".part.output.mkv").exists(), "{flags:?}");
    assert_eq!(
        fs::read(fixture.0.join("output.mkv")).unwrap(),
        b"encoded",
        "{flags:?}"
    );
    let calls = fs::read_to_string(fixture.0.join("probe-calls")).unwrap();
    assert_eq!(calls.lines().count(), 2, "{flags:?}");
    assert!(
        calls
            .lines()
            .all(|line| line.contains("-show_data_hash sha256")),
        "{flags:?}: {calls}"
    );
}

#[test]
fn short_verify_flag_warns_and_publishes_the_output() {
    assert_verification_warning_is_published(["-v", "--copy"]);
}

#[test]
fn long_verify_flag_warns_and_publishes_the_output() {
    assert_verification_warning_is_published(["--copy", "--verify"]);
}

#[track_caller]
fn assert_cover_verification(case: &str, expected: Option<&str>) {
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
        "../../../yog-core/src/ffmpeg/test_pixel_formats.json"
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
    command.args(["--copy", "--verify", "--overwrite-output"]);
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
    assert!(!fixture.0.join(".part.output.mkv").exists());
}

#[test]
fn verification_accepts_planned_cover_tags_and_reordered_streams() {
    assert_cover_verification("ok", None);
}

#[test]
fn verification_warns_about_missing_cover_filename() {
    assert_cover_verification("missing filename", Some("metadata \"filename\""));
}

#[test]
fn verification_warns_about_wrong_cover_mime_type() {
    assert_cover_verification("wrong mime", Some("metadata \"mimetype\""));
}

#[test]
fn verification_warns_about_changed_original_cover_title() {
    assert_cover_verification("changed original title", Some("metadata \"title\""));
}

#[test]
fn verification_warns_about_unplanned_cover_tags() {
    assert_cover_verification("unplanned tag", Some("metadata \"extra\""));
}

#[test]
fn verification_warns_about_changed_cover_content() {
    assert_cover_verification("changed image", Some("cover stream #8 -> #1: packet data"));
}

#[test]
fn verification_warns_about_lost_covers() {
    assert_cover_verification("lost cover", Some("cover stream #5: missing"));
}

#[test]
fn verification_warns_about_covers_converted_to_video() {
    assert_cover_verification("cover as video", Some("video stream #2: added"));
}

#[test]
fn verification_warns_about_changed_audio_timing() {
    assert_cover_verification(
        "audio timing",
        Some("audio stream #2 -> #0: packet presentation"),
    );
}

#[test]
fn verification_warns_about_unplanned_mp4_cover_tags_and_timing() {
    assert_cover_verification("unplanned mp4 tags", Some("metadata \"FILENAME\": added"));
}

#[track_caller]
fn assert_incomplete_verification(failure: &str) {
    let fixture = Fixture::new("for last do :; done; printf encoded > \"$last\"");
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
        .args(["--copy", "--verify", "--overwrite-output"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{failure}: {stderr}");
    assert!(
        stderr.contains("verification incomplete"),
        "{failure}: {stderr}"
    );
    assert!(stderr.contains("broken"), "{failure}: {stderr}");
    assert!(
        stdout.contains("complete: output.mkv"),
        "{failure}: {stdout}"
    );
    assert!(!fixture.0.join(".part.output.mkv").exists(), "{failure}");
    assert_eq!(
        fs::read(fixture.0.join("output.mkv")).unwrap(),
        b"encoded",
        "{failure}"
    );
}

#[test]
fn failed_verification_probe_warns_and_publishes_the_output() {
    assert_incomplete_verification("metadata");
}

#[test]
fn failed_verification_packet_probe_warns_and_publishes_the_output() {
    assert_incomplete_verification("packets");
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
        case "$*" in *.part.output.mkv*) ;; *) printf '1, 0, 0, 1, 1, hash\n' ;; esac
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
        "../../../yog-core/src/ffmpeg/test_pixel_formats.json"
    ))
    .unwrap()["pixel_formats"]
        .clone();
    fs::write(fixture.0.join("input.json"), media.to_string()).unwrap();
    fs::write(fixture.0.join("output.json"), media.to_string()).unwrap();
    fs::write(fixture.0.join("output.mkv"), b"previous").unwrap();

    let result = fixture
        .command()
        .args([
            "--encode-x264",
            "--verify",
            "--overwrite-output",
            "--report",
            "report.jsonl",
        ])
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
    assert!(!fixture.0.join(".part.output.mkv").exists());
    assert_eq!(
        read_report(&fixture, "report.jsonl")[0]["transcode"]["result"]["phase"],
        "verifying"
    );
}
