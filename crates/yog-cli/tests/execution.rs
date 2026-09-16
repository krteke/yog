#![cfg(unix)]

use std::{
    fs,
    io::Read,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static PROCESS_CONTROL_TEST: Mutex<()> = Mutex::new(());

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
            .env_remove("RUST_LOG")
            .args([
                "transcode",
                "-i",
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

    fn analysis_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yog"));
        command
            .current_dir(&self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .env_remove("RUST_LOG")
            .args([
                "-i",
                "input.mkv",
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
                "transcode",
                "-i",
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
fn quiet_disables_runtime_terminal_output_on_success_and_failure() {
    for (ffmpeg, expected_status, output_exists) in [
        (
            "for last do :; done; printf diagnostic >&2; printf encoded > \"$last\"",
            0,
            true,
        ),
        ("printf diagnostic >&2; exit 9", 1, false),
    ] {
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
        assert!(!fixture.0.join("output.mkv.part").exists());
    }
}

#[test]
fn prediction_samples_the_real_plan_without_a_video_output() {
    let fixture = Fixture::new(
        r#"printf '%s\n' "$*" >> ffmpeg-commands
filter=''
cover=false
image=false
for argument do
    case "$argument" in
        encoder=libx264)
            printf '%s\n' 'Encoder libx264 [test]' '    Supported pixel formats: yuv420p'
            exit 0
            ;;
        *libvmaf=*) filter=$argument ;;
        -attach) cover=true ;;
        image2pipe) image=true ;;
    esac
done
if test -n "$filter"; then
    metrics=${filter#*log_path=\'}
    metrics=${metrics%%\':shortest=*}
    printf '%s' '{"frames":[{"frameNum":0,"metrics":{"vmaf":96.5,"float_ssim":0.99,"psnr_y":42.0}}]}' > "$metrics"
    exit 0
fi
if test "$image" = true; then
    printf cover
    exit 0
fi
for last do :; done
if test "$cover" = true; then
    printf 12345678901234567890 > "$last"
    printf 'out_time_us=1000000\nspeed=1x\nprogress=end\n'
else
    printf 123456789012345 > "$last"
    printf 'out_time_us=2000000\nspeed=4x\nprogress=end\n'
fi"#,
    );
    fixture.tool(
        "ffprobe",
        r#"packets=false
for argument do
    test "$argument" = -show_packets && packets=true
done
for last do :; done
if test "$packets" = true; then
    printf '%s' '{"packets":[{"stream_index":0,"size":"10"}]}'
    exit 0
fi
case "$last" in
    input.mkv|input-dir/clip.mkv)
        printf '%s' '{"streams":[{"index":0,"codec_type":"video","width":320,"height":180,"pix_fmt":"yuv420p"},{"index":1,"codec_type":"video","codec_name":"png","disposition":{"attached_pic":1}},{"index":2,"codec_type":"attachment","extradata_size":7}],"format":{"format_name":"matroska,webm","duration":"4"},"pixel_formats":[{"name":"yuv420p","nb_components":3,"log2_chroma_w":1,"log2_chroma_h":1,"flags":{"rgb":0,"alpha":0,"palette":0,"hwaccel":0},"components":[{"bit_depth":8},{"bit_depth":8},{"bit_depth":8}]}]}'
        ;;
    *)
        if test "$(wc -c < "$last")" = 20; then
            printf '%s' '{"streams":[{"index":0,"codec_type":"video"},{"index":1,"codec_type":"attachment","extradata_size":5}],"format":{"duration":"1"}}'
        else
            printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"duration":"1"}}'
        fi
        ;;
esac"#,
    );

    let result = fixture
        .analysis_command()
        .args(["predict", "--encode-x264"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert!(!fixture.0.join("output.mkv").exists());
    assert!(!fixture.0.join("output.mkv.part").exists());
    let commands = fs::read_to_string(fixture.0.join("ffmpeg-commands")).unwrap();
    assert!(commands.contains("-ss 0.000000000"));
    assert!(commands.contains("-ss 2.000000000"));
    assert!(commands.contains("-t 2.000000000"));
    assert_eq!(
        commands
            .lines()
            .filter(|command| command.contains("-f image2pipe"))
            .count(),
        1
    );
    assert_eq!(
        commands
            .lines()
            .filter(|command| command.contains("-attach"))
            .count(),
        1
    );
    assert_eq!(
        commands
            .lines()
            .filter(|command| command.contains("-progress pipe:1"))
            .count(),
        2
    );
    assert!(commands.contains("crop=w=320:h=180:x=0:y=0:exact=1"));
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.contains("speed 2.667x | time 1.5s"), "{stdout}");
    assert!(stdout.contains("(840.0% of input)"), "{stdout}");
    assert!(stdout.contains(" | VMAF 96.500"));
    assert!(!stdout.contains("sample range"));
    assert!(!stderr.contains("executing command:"));

    let verbose = fixture
        .analysis_command()
        .args(["predict", "--encode-x264", "--verbose"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&verbose.stdout);
    let stderr = String::from_utf8_lossy(&verbose.stderr);
    assert!(verbose.status.success(), "{stderr}");
    assert!(stderr.contains("executing command:"));
    assert!(stdout.contains("sample range"));
    assert!(stdout.contains("sample #1:"));
    assert!(stdout.contains("sample #2:"));

    fs::create_dir(fixture.0.join("input-dir")).unwrap();
    fs::write(fixture.0.join("input-dir/clip.mkv"), b"input").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_yog"));
    let recursive = command
        .current_dir(&fixture.0)
        .env("XDG_CONFIG_HOME", &fixture.0)
        .env_remove("RUST_LOG")
        .args([
            "predict",
            "-i",
            "input-dir",
            "--recursive",
            "--encode-x264",
            "--ffprobe",
            "./ffprobe",
            "--ffmpeg",
            "./ffmpeg",
            "--timeout",
            "3",
        ])
        .output()
        .unwrap();
    assert!(recursive.status.success(), "{:?}", recursive.stderr);
    assert!(
        String::from_utf8_lossy(&recursive.stdout)
            .ends_with("batch summary: total 1 | succeeded 1 | failed 0\n")
    );
    assert!(!fixture.0.join("output").exists());
}

#[test]
fn emulation_writes_parameterized_charts_directly_at_the_configured_size() {
    let fixture = Fixture::new(
        r#"printf '%s\n' "$*" >> ffmpeg-commands
filter=''
for argument do
    case "$argument" in
        encoder=libsvtav1)
            printf '%s\n' 'Encoder libsvtav1 [test]' '    Supported pixel formats: yuv420p'
            exit 0
            ;;
        *libvmaf=*) filter=$argument ;;
    esac
done
if test -n "$filter"; then
    metrics=${filter#*log_path=\'}
    metrics=${metrics%%\':shortest=*}
    printf '%s' '{"frames":[{"frameNum":0,"metrics":{"vmaf":96.5,"float_ssim":0.99,"psnr_y":42.0}}]}' > "$metrics"
    exit 0
fi
for last do :; done
printf 12345678901234567890 > "$last"
printf 'out_time_us=1000000\nspeed=1x\nprogress=end\n'"#,
    );
    fixture.tool(
        "ffprobe",
        r#"packets=false
for argument do
    test "$argument" = -show_packets && packets=true
done
for last do :; done
if test "$packets" = true; then
    printf '%s' '{"packets":[{"stream_index":0,"size":"10"}]}'
elif test "$last" = input.mkv; then
    printf '%s' '{"streams":[{"index":0,"codec_type":"video","width":320,"height":180,"pix_fmt":"yuv420p"}],"format":{"format_name":"matroska,webm","duration":"1"},"pixel_formats":[{"name":"yuv420p","nb_components":3,"log2_chroma_w":1,"log2_chroma_h":1,"flags":{"rgb":0,"alpha":0,"palette":0,"hwaccel":0},"components":[{"bit_depth":8},{"bit_depth":8},{"bit_depth":8}]}]}'
else
    printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"duration":"1"}}'
fi"#,
    );
    fs::write(
        fixture.0.join("emulation.toml"),
        "[emulation]\nwidth = 900\nheight = 500\n",
    )
    .unwrap();

    let result = fixture
        .analysis_command()
        .args([
            "--config",
            "emulation.toml",
            "emulate",
            "--png",
            "quality.png",
            "--svg",
            "quality.svg",
            "--range",
            "20,22",
            "--encode-svt-av1",
            "--preset",
            "6",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert!(
        fs::read(fixture.0.join("quality.png"))
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
    let svg = fs::read_to_string(fixture.0.join("quality.svg")).unwrap();
    for label in [
        "<svg",
        "width=\"900\" height=\"500\"",
        "SVT-AV1 / Software / Preset 6",
        "CRF value",
        "VMAF score",
        "Estimated size",
        "MiB",
    ] {
        assert!(svg.contains(label), "missing {label:?} in SVG");
    }
    assert!(!fixture.0.join("output.mkv").exists());
    assert!(fs::read_dir(&fixture.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".part")
    }));

    let commands = fs::read_to_string(fixture.0.join("ffmpeg-commands")).unwrap();
    for quality in 20..=22 {
        assert_eq!(
            commands
                .lines()
                .filter(|command| command.contains(&format!("-crf:v {quality}")))
                .count(),
            1,
            "{commands}",
        );
    }
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.contains("CRF 20..=22"), "{stdout}");
    assert!(stdout.contains("PNG quality.png"), "{stdout}");
    assert!(stdout.contains("SVG quality.svg"), "{stdout}");
}

#[test]
fn published_transcode_calculates_full_or_subsampled_vmaf() {
    for (vmaf_args, expected_mode) in [
        (vec!["--vmaf"], "full"),
        (vec!["--vmaf=7"], "n_subsample=7"),
    ] {
        let fixture = Fixture::new(
            r#"printf '%s\n' "$*" >> ffmpeg-commands
filter=''
for argument do
    case "$argument" in *libvmaf=*) filter=$argument ;; esac
done
if test -n "$filter"; then
    test -f output.mkv || exit 20
    test ! -e output.mkv.part || exit 21
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
            .args(vmaf_args)
            .arg("--copy")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{stderr}");
        assert_eq!(fs::read(fixture.0.join("output.mkv")).unwrap(), b"encoded");
        assert!(!fixture.0.join("output.mkv.part").exists());
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
        if expected_mode == "full" {
            assert!(!vmaf.contains("n_subsample="), "{vmaf}");
        } else {
            assert!(vmaf.contains(":n_subsample=7"), "{vmaf}");
        }
    }
}

#[test]
fn requested_vmaf_failure_does_not_fail_or_roll_back_the_transcode() {
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
    for verbose in [false, true] {
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
        assert!(!fixture.0.join("output.mkv.part").exists());
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
    assert!(String::from_utf8_lossy(&result.stdout).contains("complete:"));
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
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("warning: skipping input/nested/notes.md because ffprobe failed"));
    assert!(stderr.contains("NOT-MEDIA"));
    assert!(stdout.contains("batch summary: total 2 | succeeded 2 | failed 0"));
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
    assert!(!fixture.0.join("output/bad.mkv.part").exists());
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
        stderr.contains("failed: input/bad.mkv: transcode failed"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("BAD-TRANSCODE").count(), 1, "{stderr}");
    assert!(
        stdout.ends_with("batch summary: total 3 | succeeded 2 | failed 1\n"),
        "{stdout}"
    );
}

#[test]
fn cancelling_recursive_mode_stops_scheduling_and_prints_the_summary() {
    let _serial = PROCESS_CONTROL_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new(
        r#"previous=''
source=''
for argument do
    if test "$previous" = -i; then source=$argument; fi
    previous=$argument
done
printf '%s\n' "$source" >> started
if test ! -e completed-one; then
    touch completed-one
    printf encoded > "$argument"
    exit 0
fi
printf '%s' "$$" > child-pid
while :; do :; done"#,
    );
    fs::create_dir(fixture.0.join("input")).unwrap();
    for input in ["first.mkv", "second.mkv", "third.mkv"] {
        fs::write(fixture.0.join("input").join(input), b"video").unwrap();
    }

    let mut child = Running(
        fixture
            .recursive_command("output")
            .arg("--copy")
            .process_group(0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let started = Instant::now();
    while !fixture.0.join("child-pid").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "second batch task did not start"
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
            "batch cancellation did not complete"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();

    assert_eq!(status.code(), Some(130));
    assert_eq!(
        fs::read_to_string(fixture.0.join("started"))
            .unwrap()
            .lines()
            .count(),
        2,
    );
    assert_eq!(fs::read_dir(fixture.0.join("output")).unwrap().count(), 1);
    assert!(
        stdout.ends_with(
            "batch summary: total 3 | succeeded 1 | failed 0 | not processed 2 | cancelled\n"
        ),
        "{stdout}"
    );
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
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{stderr}");
        assert!(stderr.contains("audio stream #8: missing"), "{stderr}");
        assert!(stderr.contains("metadata \"title\""), "{stderr}");
        assert!(stdout.contains("complete: output.mkv"), "{stdout}");
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
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{stderr}");
        assert!(stderr.contains("verification incomplete"), "{stderr}");
        assert!(stderr.contains("broken"), "{stderr}");
        assert!(stdout.contains("complete: output.mkv"));
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
        assert!(!String::from_utf8_lossy(&result.stdout).contains("complete:"));
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
fn timeout_reaps_child_and_cleans_part() {
    let _serial = PROCESS_CONTROL_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = Fixture::new(
        r#"printf '%s' "$$" > child-pid
for last do :; done
printf partial > "$last"
printf 'diagnostic-data\n' >&2
while :; do :; done"#,
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_yog"));
    let mut child = Running(
        command
            .current_dir(&fixture.0)
            .env("XDG_CONFIG_HOME", &fixture.0)
            .args([
                "transcode",
                "-i",
                "input.mkv",
                "-o",
                "output.mkv",
                "--ffprobe",
                "./ffprobe",
                "--ffmpeg",
                "./ffmpeg",
                "--timeout",
                "1",
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
fn cancellation_keeps_the_output_state_owned_by_each_completed_phase() {
    let _serial = PROCESS_CONTROL_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for phase in [
        "ffprobe",
        "ffmpeg",
        "encoder-help",
        "cover-extraction",
        "cover-encoding",
        "verify",
        "verify-packets",
        "vmaf",
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
        } else if phase == "vmaf" {
            fixture.tool(
                "ffmpeg",
                &format!(
                    "case \"$*\" in *libvmaf=*) {blocking} ;; esac\nfor last do :; done\nprintf encoded > \"$last\""
                ),
            );
            fixture.tool(
                "ffprobe",
                r#"for last do :; done
case "$last" in
    input.mkv) printf '%s' '{"streams":[{"index":0,"codec_type":"video","width":320,"height":180}],"format":{"format_name":"matroska,webm","duration":"1"}}' ;;
    *) printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"duration":"1"}}' ;;
esac"#,
            );
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
        let mut command = fixture.command();
        command.env("TMPDIR", &temporary).arg("-O");
        if phase == "vmaf" {
            command.args(["--vmaf", "--copy"]);
        } else {
            command.args([
                "--verify",
                if phase == "encoder-help" {
                    "--encode-x264"
                } else {
                    "--copy"
                },
            ]);
        }
        let mut child = Running(
            command
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
        let expected: &[u8] = if phase == "vmaf" {
            b"encoded"
        } else {
            b"original"
        };
        assert_eq!(
            fs::read(fixture.0.join("output.mkv")).unwrap(),
            expected,
            "{phase}"
        );
        let pid = fs::read_to_string(fixture.0.join("child-pid")).unwrap();
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    }
}
