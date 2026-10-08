use super::super::support::read_report;
use super::{command_quality_count, measured_emulation_fixture};
use std::fs;

#[test]
fn emulation_candidates_are_measured_and_reported_individually() {
    use serde_json::Value;

    #[track_caller]
    fn assert_point(row: &Value, candidate_index: u64, encoder: &str, preset: &str, quality: u64) {
        let record = &row["emulate"];
        assert_eq!(record["result"]["status"], "success");
        assert_eq!(record["result"]["message"], Value::Null);
        assert_eq!(record["candidate_index"], candidate_index);
        assert_eq!(record["decoding"], "software");
        assert_eq!(record["source"]["bytes"], 20);
        assert_eq!(record["quality"]["vmaf"]["value"], 96.5);
        assert_eq!(record["video"]["encoder"], encoder);
        assert_eq!(record["video"]["preset"], preset);
        assert_eq!(record["video"]["rate"]["value"], quality);
        assert_eq!(record["video"]["device"], Value::Null);
        let png = record["outputs"]["png"].as_str().unwrap();
        assert!(png.starts_with('/'), "{png}");
        assert!(png.ends_with("candidates.png"), "{png}");
    }

    let fixture = measured_emulation_fixture();

    let result = fixture
        .analysis_command()
        .args([
            "emulate",
            "--png",
            "candidates.png",
            "--svg",
            "candidates.svg",
            "--candidate",
            "software:libx264:medium:20-21",
            "--candidate",
            "software:libsvtav1:7:30-31",
            "--report",
            "report.jsonl",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");

    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 4, "{records:?}");
    assert_point(&records[0], 1, "libx264", "medium", 20);
    assert_point(&records[1], 1, "libx264", "medium", 21);
    assert_point(&records[2], 2, "libsvtav1", "7", 30);
    assert_point(&records[3], 2, "libsvtav1", "7", 31);

    let commands = fs::read_to_string(fixture.0.join("ffmpeg-commands")).unwrap();
    assert_eq!(
        command_quality_count(&commands, "libx264", 20),
        1,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libx264", 21),
        1,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libx264", 30),
        0,
        "{commands}"
    );
    assert_eq!(
        command_quality_count(&commands, "libsvtav1", 20),
        0,
        "{commands}"
    );
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

    let svg = fs::read_to_string(fixture.0.join("candidates.svg")).unwrap();
    assert!(svg.contains("Candidates 1 / 2"), "{svg}");
    assert!(svg.contains("CRF value"), "{svg}");
    assert!(svg.contains("1: x264 / Software / Preset medium"), "{svg}");
    assert!(svg.contains("2: SVT-AV1 / Software / Preset 7"), "{svg}");
    assert_eq!(
        svg.matches("x264 / Software / Preset medium").count(),
        1,
        "{svg}"
    );
    assert_eq!(
        svg.matches("SVT-AV1 / Software / Preset 7").count(),
        1,
        "{svg}"
    );
    assert!(
        svg.contains("Solid: VMAF    Dashed: estimated size"),
        "{svg}"
    );
    assert!(!svg.contains("<circle"), "{svg}");
    assert!(svg.contains("stroke=\"#16A34A\""), "{svg}");
    assert!(svg.contains("stroke=\"#2563EB\""), "{svg}");

    let result = fixture
        .analysis_command()
        .args([
            "emulate",
            "--svg",
            "mixed.svg",
            "--candidate",
            "software:libx264:medium:20",
            "--candidate",
            "software:hevc_nvenc:p5:25",
        ])
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result.stderr);
    let svg = fs::read_to_string(fixture.0.join("mixed.svg")).unwrap();
    assert!(svg.contains("CRF / CQ value"), "{svg}");
    assert!(
        svg.contains("2: HEVC NVENC / Software / Preset p5"),
        "{svg}"
    );
}
