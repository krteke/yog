use super::support::{Fixture, read_report};
use std::fs;

#[test]
fn transcode_reports_measured_result_and_stream_facts() {
    use serde_json::Value;

    let fixture = Fixture::new(
        r#"for last do :; done
printf 1234567 > "$last"
printf 'frame=25\nfps=25.0\nspeed=2x\nout_time_us=1000000\ntotal_size=7\nprogress=end\n'"#,
    );
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
case "$last" in
    input.mkv)
        printf '%s' '{"streams":[{"index":0,"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","width":320,"height":180,"avg_frame_rate":"25/1","bit_rate":"1000"},{"index":1,"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000"},{"index":2,"codec_type":"video","codec_name":"png","disposition":{"attached_pic":1}}],"format":{"format_name":"matroska,webm","duration":"1","start_time":"0"}}'
        ;;
    *)
        printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"duration":"1"}}'
        ;;
esac"#,
    );

    let result = fixture
        .command()
        .args(["--copy", "--report", "report.jsonl"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");

    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0]["transcode"];
    assert_eq!(record["result"]["status"], "success");
    assert!(record["input"].as_str().unwrap().starts_with('/'));
    assert!(record["input"].as_str().unwrap().ends_with("input.mkv"));
    assert!(record["output"].as_str().unwrap().ends_with("output.mkv"));
    assert_eq!(record["container"], "mkv");
    assert_eq!(record["video"]["action"], "copy");
    assert_eq!(record["video"]["encoder"], Value::Null);
    assert_eq!(record["video"]["rate"], Value::Null);
    assert_eq!(record["decoding"], "software");
    assert_eq!(record["source"]["bytes"], 5);
    assert_eq!(record["source"]["duration_seconds"], 1.0);
    assert_eq!(
        record["source"]["streams"],
        serde_json::json!({"video":1,"audio":1,"subtitle":0,"attachment":0,"cover":1,"other":0})
    );
    assert_eq!(record["source"]["video"]["codec"], "h264");
    assert_eq!(record["source"]["video"]["width"], 320);
    assert_eq!(record["source"]["video"]["frame_rate"], "25/1");
    assert_eq!(record["source"]["video"]["bit_rate"], "1000");
    assert_eq!(
        record["source"]["audio"],
        serde_json::json!([{"codec":"aac","channels":2,"sample_rate":"48000"}])
    );
    assert_eq!(record["metrics"]["bytes"], 7);
    assert_eq!(record["metrics"]["duration_seconds"], 1.0);
    assert_eq!(record["metrics"]["size_percent"], 140.0);
    assert_eq!(record["metrics"]["frames"], 25);
    assert_eq!(record["metrics"]["fps"], 25.0);
    assert!(record["metrics"]["elapsed_seconds"].as_f64().unwrap() > 0.0);
    assert!(record["metrics"]["speed"].as_f64().unwrap() > 0.0);
    assert_eq!(record["verify"], Value::Null);
    assert_eq!(record["vmaf"], Value::Null);
    assert_eq!(record["result"]["message"], Value::Null);
}

#[test]
fn recursive_reports_include_skipped_inputs_and_failures() {
    let fixture = Fixture::new(
        r#"previous=''
source=''
for argument do
    if test "$previous" = -i; then source=$argument; fi
    previous=$argument
done
case "$source" in
    input/bad.mkv) printf 'BAD-TRANSCODE\n' >&2; exit 9 ;;
esac
for last do :; done
printf encoded > "$last"
printf 'frame=1\nout_time_us=1000000\nspeed=1x\nprogress=end\n'"#,
    );
    fs::create_dir(fixture.0.join("input")).unwrap();
    for input in ["first.mkv", "bad.mkv", "last.mkv"] {
        fs::write(fixture.0.join("input").join(input), b"video").unwrap();
    }
    fs::write(fixture.0.join("input/notes.md"), b"notes").unwrap();
    fixture.tool(
        "ffprobe",
        r#"for last do :; done
case "$last" in
    input/notes.md) printf 'NOT-MEDIA\n' >&2; exit 1 ;;
esac
printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"matroska,webm","duration":"1"}}'"#,
    );

    let result = fixture
        .recursive_command("output")
        .args(["--copy", "--report", "report.jsonl"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1), "{:?}", result.stderr);

    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 4, "{records:?}");

    let skipped: Vec<_> = records
        .iter()
        .filter_map(|row| row["skipped"].as_object())
        .collect();
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0]["input"].as_str().unwrap().ends_with("notes.md"));
    assert_eq!(skipped[0]["result"]["reason"], "probe_failed");
    let reason = skipped[0]["result"]["message"].as_str().unwrap();
    assert!(reason.contains("probe failed"), "{reason}");

    let items: Vec<_> = records
        .iter()
        .filter_map(|row| row["transcode"].as_object())
        .collect();
    assert_eq!(items.len(), 3);
    assert_eq!(
        items
            .iter()
            .filter(|item| item["result"]["status"] == "success")
            .count(),
        2
    );
    let failed: Vec<_> = items
        .iter()
        .filter(|item| item["result"]["status"] == "failed")
        .collect();
    assert_eq!(failed.len(), 1);
    assert!(failed[0]["input"].as_str().unwrap().ends_with("bad.mkv"));
    assert_eq!(failed[0]["result"]["phase"], "transcoding");
    assert!(
        failed[0]["result"]["message"]
            .as_str()
            .unwrap()
            .contains("transcode failed")
    );
    let succeeded: Vec<_> = items
        .iter()
        .filter(|item| item["result"]["status"] == "success")
        .collect();
    assert_eq!(succeeded.len(), 2);
    assert_eq!(succeeded[0]["metrics"]["bytes"], 7);
    assert_eq!(succeeded[0]["source"]["bytes"], 5);
    assert_eq!(succeeded[1]["metrics"]["bytes"], 7);
    assert_eq!(succeeded[1]["source"]["bytes"], 5);
}

#[test]
fn reports_survive_task_failures() {
    let fixture = Fixture::new("printf 'BOOM\n' >&2; exit 7");
    fs::write(fixture.0.join("report.jsonl"), b"stale line\n").unwrap();
    let result = fixture
        .command()
        .args(["--copy", "--report", "report.jsonl"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1), "{:?}", result.stderr);
    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["transcode"]["result"]["status"], "failed");
    assert_eq!(records[0]["transcode"]["result"]["phase"], "transcoding");
    assert_eq!(records[0]["transcode"]["result"]["timed_out"], false);
    assert!(
        records[0]["transcode"]["result"]["message"]
            .as_str()
            .unwrap()
            .contains("transcode failed"),
        "{}",
        records[0]
    );
}
