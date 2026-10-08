use super::support::{Fixture, read_report};
use std::fs;

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

    let assert_output = |extension| {
        assert_eq!(
            fs::read(fixture.0.join(format!("output/video.{extension}"))).unwrap(),
            b"encoded",
            "{extension}"
        );
    };
    assert_output("mkv");
    assert_output("webm");
    assert_output("mp4");
    assert_output("mov");
    assert_output("m4a");
    assert_output("3gp");
    assert_output("3g2");
    assert_output("f4v");
    assert_output("ismv");
    assert_output("psp");
    assert_output("ts");
    assert_output("m2ts");
    assert_output("avi");
    assert_output("flv");
    assert_output("asf");
    assert_output("wmv");
    assert_output("mpg");
    assert_output("mpeg");
    assert_output("vob");
    assert_output("ogg");
    assert_output("ogv");

    let commands = fs::read_to_string(fixture.0.join("commands")).unwrap();
    let assert_muxer = |muxer, extension| {
        assert!(
            commands
                .lines()
                .any(|line| line == format!("{muxer} output/.part.video.{extension}")),
            "missing {muxer} mapping for {extension}:\n{commands}"
        );
    };
    assert_muxer("matroska", "mkv");
    assert_muxer("webm", "webm");
    assert_muxer("mp4", "mp4");
    assert_muxer("mov", "mov");
    assert_muxer("ipod", "m4a");
    assert_muxer("3gp", "3gp");
    assert_muxer("3g2", "3g2");
    assert_muxer("f4v", "f4v");
    assert_muxer("ismv", "ismv");
    assert_muxer("psp", "psp");
    assert_muxer("mpegts", "ts");
    assert_muxer("mpegts", "m2ts");
    assert_muxer("avi", "avi");
    assert_muxer("flv", "flv");
    assert_muxer("asf", "asf");
    assert_muxer("asf", "wmv");
    assert_muxer("mpeg", "mpg");
    assert_muxer("mpeg", "mpeg");
    assert_muxer("vob", "vob");
    assert_muxer("ogg", "ogg");
    assert_muxer("ogv", "ogv");
}

#[test]
fn recursive_mode_skips_unsupported_input_containers_and_records_them() {
    let fixture = Fixture::new(
        r#"for last do :; done
printf encoded > "$last""#,
    );
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/cover.jpg"), b"image").unwrap();
    fs::write(fixture.0.join("input/first.mkv"), b"video").unwrap();
    fs::write(fixture.0.join("input/last.mkv"), b"video").unwrap();
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
case "$last" in
    *.jpg) format=image2 ;;
    *) format=matroska,webm ;;
esac
printf '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"%s","duration":"1"}}' "$format""#,
    );

    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--report", "report.jsonl"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert_eq!(
        fs::read(fixture.0.join("output/first.mkv")).unwrap(),
        b"encoded"
    );
    assert_eq!(
        fs::read(fixture.0.join("output/last.mkv")).unwrap(),
        b"encoded"
    );
    assert!(!fixture.0.join("output/cover.jpg").exists());
    assert!(!fixture.0.join("output/.part.cover.jpg").exists());
    assert!(
        stderr.contains("warning: skipping input/cover.jpg"),
        "{stderr}"
    );
    assert!(
        stderr.contains("cannot use input format Some(\"image2\")"),
        "{stderr}"
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.ends_with("batch summary: total 3 | succeeded 2 | failed 0 | skipped 1\n"),
        "{stdout}"
    );

    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 3, "{records:?}");
    let skipped: Vec<_> = records
        .iter()
        .filter_map(|row| row["skipped"].as_object())
        .collect();
    assert_eq!(skipped.len(), 1, "{records:?}");
    assert!(skipped[0]["input"].as_str().unwrap().ends_with("cover.jpg"));
    assert_eq!(skipped[0]["result"]["reason"], "unsupported_container");
    assert!(
        skipped[0]["result"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot use input format Some(\"image2\")")
    );
    assert_eq!(
        records
            .iter()
            .filter(|row| row["transcode"]["result"]["status"] == "success")
            .count(),
        2
    );
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
    fs::write(fixture.0.join("input/unrelated.mkv"), b"video").unwrap();
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
    assert!(!fixture.0.join("output").exists());
}

#[test]
fn recursive_mode_does_not_treat_a_same_named_subtitle_as_an_output_collision() {
    let fixture = Fixture::new("for last do :; done; printf encoded > \"$last\"");
    fs::create_dir(fixture.0.join("input")).unwrap();
    fs::write(fixture.0.join("input/movie.mkv"), b"video").unwrap();
    fs::write(fixture.0.join("input/movie.srt"), b"subtitle").unwrap();
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
case "$last" in
    *.srt) streams='[{"index":0,"codec_type":"subtitle"}]' ;;
    *) streams='[{"index":0,"codec_type":"video"}]' ;;
esac
printf '{"streams":%s,"format":{"format_name":"matroska,webm","duration":"1"}}' "$streams""#,
    );

    let result = fixture
        .recursive_command("output")
        .args(["-C", "webm", "--copy", "--report", "report.jsonl"])
        .output()
        .unwrap();

    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(
        fs::read(fixture.0.join("output/movie.webm")).unwrap(),
        b"encoded"
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .ends_with("batch summary: total 2 | succeeded 1 | failed 0 | skipped 1\n")
    );
    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(
        records
            .iter()
            .filter(|record| record["skipped"].is_object())
            .count(),
        1
    );
}
