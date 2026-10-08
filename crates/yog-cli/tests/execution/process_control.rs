use super::support::{Fixture, read_report};
use std::{
    fs,
    io::Read,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

static PROCESS_CONTROL_TEST: Mutex<()> = Mutex::new(());

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
            "batch summary: total 3 | succeeded 1 | failed 0 | skipped 0 | not processed 2 | cancelled\n"
        ),
        "{stdout}"
    );
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
    let mut child = Running(
        fixture
            .cli()
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
                "--report",
                "report.jsonl",
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
    let rows = read_report(&fixture, "report.jsonl");
    assert_eq!(rows[0]["transcode"]["result"]["phase"], "transcoding");
    assert_eq!(rows[0]["transcode"]["result"]["timed_out"], true);
    assert!(!fixture.0.join(".part.output.mkv").exists());
    assert!(!fixture.0.join("output.mkv").exists());
    let pid = fs::read_to_string(fixture.0.join("child-pid")).unwrap();
    assert!(
        !PathBuf::from(format!("/proc/{pid}")).exists(),
        "FFmpeg was not reaped"
    );
}

#[track_caller]
fn assert_cancelled_phase(phase: &str) {
    let _serial = PROCESS_CONTROL_TEST
        .lock()
        .unwrap_or_else(|error| error.into_inner());
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
        command.args(["--vmaf", "--copy", "--report", "report.jsonl"]);
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
    assert!(!fixture.0.join(".part.output.mkv").exists());
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
    if phase == "vmaf" {
        let records = read_report(&fixture, "report.jsonl");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["transcode"]["result"]["status"], "success");
        assert_eq!(records[0]["transcode"]["vmaf"]["error"], "cancelled");
    }
    let pid = fs::read_to_string(fixture.0.join("child-pid")).unwrap();
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
}

#[test]
fn cancelling_input_probe_preserves_the_previous_output() {
    assert_cancelled_phase("ffprobe");
}

#[test]
fn cancelling_transcode_preserves_the_previous_output() {
    assert_cancelled_phase("ffmpeg");
}

#[test]
fn cancelling_encoder_help_preserves_the_previous_output() {
    assert_cancelled_phase("encoder-help");
}

#[test]
fn cancelling_cover_extraction_cleans_covers_and_preserves_the_previous_output() {
    assert_cancelled_phase("cover-extraction");
}

#[test]
fn cancelling_cover_encoding_cleans_covers_and_preserves_the_previous_output() {
    assert_cancelled_phase("cover-encoding");
}

#[test]
fn cancelling_verification_preserves_the_previous_output() {
    assert_cancelled_phase("verify");
}

#[test]
fn cancelling_packet_verification_preserves_the_previous_output() {
    assert_cancelled_phase("verify-packets");
}

#[test]
fn cancelling_vmaf_keeps_the_published_output_and_reports_success() {
    assert_cancelled_phase("vmaf");
}
