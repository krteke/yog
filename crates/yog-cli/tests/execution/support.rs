use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

// These are isolated test fixtures, not the application's output workflow.
pub(super) struct Fixture(pub(super) PathBuf);

impl Fixture {
    pub(super) fn new(ffmpeg: &str) -> Self {
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

    pub(super) fn tool(&self, name: &str, body: &str) {
        let path = self.0.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    // Cargo supplies the absolute path to this package's default binary at compile time.
    pub(super) fn cli(&self) -> Command {
        let mut command = Command::new(env!(concat!("CARGO_BIN_EXE_", env!("CARGO_PKG_NAME"))));
        command
            .current_dir(&self.0)
            .env("XDG_CONFIG_HOME", &self.0)
            .env_remove("RUST_LOG");
        command
    }

    pub(super) fn command(&self) -> Command {
        let mut command = self.cli();
        command.args([
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

    pub(super) fn analysis_command(&self) -> Command {
        let mut command = self.cli();
        command.args([
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

    pub(super) fn recursive_command(&self, output: &str) -> Command {
        let mut command = self.cli();
        command.args([
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

pub(super) fn read_report(fixture: &Fixture, name: &str) -> Vec<serde_json::Value> {
    fs::read_to_string(fixture.0.join(name))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
