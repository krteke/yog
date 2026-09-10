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
        fs::write(fixture.0.join("input"), b"input").unwrap();
        fixture.tool(
            "ffprobe",
            "printf '%s' '{\"streams\":[{\"index\":0,\"codec_type\":\"video\"}],\"format\":{\"duration\":\"1\"}}'",
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
                "input",
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
                "input",
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
fn cancellation_during_probe_or_encoding_uses_one_exit_status_and_cleans_part() {
    for phase in ["ffprobe", "ffmpeg", "encoder-help"] {
        let fixture = Fixture::new("");
        fixture.tool(
            if phase == "encoder-help" {
                "ffmpeg"
            } else {
                phase
            },
            "printf '%s' \"$$\" > child-pid; printf waiting >&2; while :; do :; done",
        );
        fs::write(fixture.0.join("output.mkv"), b"original").unwrap();
        let mut child = Running(
            fixture
                .command()
                .args([
                    "-O",
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
