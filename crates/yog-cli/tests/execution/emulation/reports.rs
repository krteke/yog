use super::super::support::read_report;
use super::{emulation_fixture, measured_emulation_fixture};

#[test]
fn emulated_points_are_reported_one_line_each() {
    use serde_json::Value;

    let fixture = measured_emulation_fixture();

    let result = fixture
        .analysis_command()
        .args([
            "emulate",
            "--png",
            "quality.png",
            "--svg",
            "quality.svg",
            "--range",
            "20-22",
            "--report",
            "report.jsonl",
            "--encode-svt-av1",
            "--preset",
            "6",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");

    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 3, "{records:?}");
    assert_eq!(
        records
            .iter()
            .map(|row| row["emulate"]["video"]["rate"]["value"].clone())
            .collect::<Vec<_>>(),
        vec![
            serde_json::json!(20),
            serde_json::json!(21),
            serde_json::json!(22)
        ],
    );

    assert_eq!(
        records
            .iter()
            .map(|row| row["emulate"]["result"]["status"].clone())
            .collect::<Vec<_>>(),
        vec![
            serde_json::json!("success"),
            serde_json::json!("success"),
            serde_json::json!("success")
        ]
    );

    let record = &records[0]["emulate"];
    assert!(record.get("predict").is_none(), "{record}");
    assert_eq!(record["result"]["message"], Value::Null);
    assert_eq!(record["container"], "mkv");
    assert_eq!(record["video"]["action"], "encode");
    assert_eq!(record["video"]["encoder"], "libsvtav1");
    assert_eq!(record["video"]["rate"]["kind"], "quality");
    assert_eq!(record["video"]["rate"]["parameter"], "CRF");
    assert_eq!(record["video"]["preset"], "6");
    assert_eq!(record["video"]["multipass"], Value::Null);
    assert_eq!(record["video"]["device"], Value::Null);
    assert_eq!(record["decoding"], "software");
    assert!(record["input"].as_str().unwrap().ends_with("input.mkv"));
    assert_eq!(record["source"]["bytes"], 20);
    assert_eq!(record["source"]["duration_seconds"], 1.0);
    assert_eq!(record["source"]["streams"]["video"], 1);
    assert_eq!(
        record["sampling"],
        serde_json::json!({
            "requested_samples": 5,
            "sample_seconds": 2.0,
            "measured_samples": 1,
            "sampled_seconds": 1.0
        })
    );
    assert_eq!(
        record["speed"],
        serde_json::json!({"value": 1.0, "low": 1.0, "high": 1.0})
    );
    assert_eq!(
        record["transcode_seconds"],
        serde_json::json!({"value": 1.0, "low": 1.0, "high": 1.0})
    );
    assert_eq!(
        record["output_bytes"],
        serde_json::json!({"value": 20, "low": 20, "high": 20})
    );
    assert_eq!(record["size_percent"], 100.0);
    assert_eq!(record["quality"]["frames"], 1);
    assert_eq!(record["quality"]["source_stream_index"], 0);
    assert_eq!(record["quality"]["vmaf"]["value"], 96.5);
    assert_eq!(record["quality"]["ssim"]["value"], 0.99);
    assert_eq!(record["quality"]["psnr_y_db"]["value"], 42.0);
    assert_eq!(record["samples"].as_array().unwrap().len(), 1);
    assert_eq!(record["samples"][0]["vmaf"], 96.5);

    let png = record["outputs"]["png"].as_str().unwrap();
    assert!(png.starts_with('/'), "{png}");
    assert!(png.ends_with("quality.png"), "{png}");
    let svg = record["outputs"]["svg"].as_str().unwrap();
    assert!(svg.starts_with('/'), "{svg}");
    assert!(svg.ends_with("quality.svg"), "{svg}");
}

#[test]
fn failed_emulation_points_do_not_discard_successful_points() {
    let fixture = emulation_fixture(
        r#"printf '%s\n' "$*" >> ffmpeg-commands
filter=''
previous=''
for argument do
    case "$argument" in
        encoder=libsvtav1)
            printf '%s\n' 'Encoder libsvtav1 [test]' '    Supported pixel formats: yuv420p'
            exit 0
            ;;
        *libvmaf=*) filter=$argument ;;
    esac
    if test "$previous" = -crf:v && test "$argument" = 21; then
        printf 'BAD-ENCODE\n' >&2
        exit 9
    fi
    previous=$argument
done
if test -n "$filter"; then
    metrics=${filter#*log_path=\'}
    metrics=${metrics%%\':shortest=*}
    printf '%s' '{"frames":[{"frameNum":0,"metrics":{"vmaf":96.5}}]}' > "$metrics"
    exit 0
fi
for last do :; done
printf 12345678901234567890 > "$last"
printf 'out_time_us=1000000\nspeed=1x\nprogress=end\n'"#,
    );

    let result = fixture
        .analysis_command()
        .args([
            "emulate",
            "--png",
            "quality.png",
            "--range",
            "20-22",
            "--report",
            "report.jsonl",
            "--encode-svt-av1",
            "--preset",
            "6",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("prediction failed at CRF 21"), "{stderr}");
    assert!(fixture.0.join("quality.png").exists());
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("points 2 succeeded, 1 failed"), "{stdout}");

    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 3, "{records:?}");
    assert_eq!(records[0]["emulate"]["result"]["status"], "success");
    assert_eq!(records[0]["emulate"]["video"]["rate"]["value"], 20);
    assert_eq!(records[1]["emulate"]["result"]["status"], "failed");
    assert_eq!(records[1]["emulate"]["result"]["phase"], "predicting");
    assert_eq!(records[1]["emulate"]["video"]["rate"]["value"], 21);
    let error = records[1]["emulate"]["result"]["message"].as_str().unwrap();
    assert!(error.contains("prediction failed at CRF 21"), "{error}");
    assert_eq!(records[1]["emulate"]["quality"], serde_json::Value::Null);
    assert_eq!(records[2]["emulate"]["result"]["status"], "success");
    assert_eq!(records[2]["emulate"]["video"]["rate"]["value"], 22);
}
