use super::{Fixture, command_quality_count, measured_emulation_fixture};
use std::fs;

fn chart_fixture() -> Fixture {
    let fixture = measured_emulation_fixture();
    fs::write(
        fixture.0.join("emulation.toml"),
        "[emulation]\nwidth = 900\nheight = 500\n",
    )
    .unwrap();
    fixture
}

#[test]
fn emulation_writes_parameterized_charts_directly_at_the_configured_size() {
    let fixture = chart_fixture();

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
            "20-22",
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
    assert!(svg.contains("<svg"), "{svg}");
    assert!(svg.contains("width=\"900\" height=\"500\""), "{svg}");
    assert!(svg.contains("SVT-AV1 / Software / Preset 6"), "{svg}");
    assert!(svg.contains("CRF value"), "{svg}");
    assert!(svg.contains("VMAF score"), "{svg}");
    assert!(svg.contains("Estimated size"), "{svg}");
    assert!(svg.contains("MiB"), "{svg}");
    assert!(svg.contains("Gray dashed: input size ("), "{svg}");
    assert!(!svg.contains("<circle"), "{svg}");
    assert!(!fixture.0.join("output.mkv").exists());
    assert!(fs::read_dir(&fixture.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".part")
    }));

    let commands = fs::read_to_string(fixture.0.join("ffmpeg-commands")).unwrap();
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 20),
        1,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 21),
        1,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 22),
        1,
        "{commands}"
    );
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.contains("candidates 1"), "{stdout}");
    assert!(stdout.contains("points 3 succeeded, 0 failed"), "{stdout}");
    assert!(stdout.contains("PNG quality.png"), "{stdout}");
    assert!(stdout.contains("SVG quality.svg"), "{stdout}");
}

#[test]
fn emulation_hides_input_size_reference_outside_the_chart_range() {
    let fixture = chart_fixture();
    fs::write(fixture.0.join("input.mkv"), b"input").unwrap();
    let result = fixture
        .analysis_command()
        .args([
            "--config",
            "emulation.toml",
            "emulate",
            "--svg",
            "outside.svg",
            "--range",
            "20,22",
            "--encode-svt-av1",
            "--preset",
            "6",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    let svg = fs::read_to_string(fixture.0.join("outside.svg")).unwrap();
    assert!(!svg.contains("Gray dashed: input size ("), "{svg}");
}

#[test]
fn emulation_measures_only_requested_sparse_qualities() {
    let fixture = chart_fixture();
    // Sparse points must not be expanded into the qualities between them.
    let result = fixture
        .analysis_command()
        .args([
            "--config",
            "emulation.toml",
            "emulate",
            "--png",
            "sparse.png",
            "--range",
            "30-31,35",
            "--encode-svt-av1",
            "--preset",
            "6",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        result.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(stdout.contains("points 3 succeeded, 0 failed"), "{stdout}");
    let commands = fs::read_to_string(fixture.0.join("ffmpeg-commands")).unwrap();
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 30),
        1,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 31),
        1,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 32),
        0,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 33),
        0,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 34),
        0,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 35),
        1,
        "{commands}"
    );
}
