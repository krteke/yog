use super::support::{Fixture, read_report};
use std::fs;

fn prediction_fixture() -> Fixture {
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
    fixture
}

#[test]
fn prediction_samples_the_real_plan_without_a_video_output() {
    let fixture = prediction_fixture();

    let result = fixture
        .analysis_command()
        .args(["predict", "--encode-x264"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert!(!fixture.0.join("output.mkv").exists());
    assert!(!fixture.0.join(".part.output.mkv").exists());
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
}

#[test]
fn verbose_prediction_reports_commands_and_individual_samples() {
    let fixture = prediction_fixture();
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
}

#[test]
fn recursive_prediction_prints_a_batch_summary_without_video_outputs() {
    let fixture = prediction_fixture();
    fs::create_dir(fixture.0.join("input-dir")).unwrap();
    fs::write(fixture.0.join("input-dir/clip.mkv"), b"input").unwrap();
    let recursive = fixture
        .cli()
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
    assert!(String::from_utf8_lossy(&recursive.stdout).ends_with(
        "batch summary: total 1 | succeeded 1 | failed 0 | skipped 0 | reused 0 | forced 0\n"
    ));
    assert!(!fixture.0.join("output").exists());
}

#[test]
fn predictions_are_reported_with_the_configuration_that_produced_them() {
    let fixture = prediction_fixture();

    let result = fixture
        .analysis_command()
        .args(["predict", "--encode-x264", "--report", "report.jsonl"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");

    let records = read_report(&fixture, "report.jsonl");
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0]["predict"];
    assert_eq!(record["result"]["status"], "success");
    assert!(record["input"].as_str().unwrap().starts_with('/'));
    assert_eq!(record["container"], "mkv");
    assert_eq!(record["video"]["encoder"], "libx264");
    assert_eq!(record["decoding"], "software");
    assert_eq!(
        record["sampling"],
        serde_json::json!({
            "requested_samples": 5,
            "sample_seconds": 2.0,
            "measured_samples": 2,
            "sampled_seconds": 4.0
        })
    );
    assert_eq!(record["source"]["bytes"], 5);
    assert_eq!(record["source"]["duration_seconds"], 4.0);
    assert_eq!(record["source"]["streams"]["video"], 1);
    assert_eq!(record["source"]["streams"]["cover"], 1);
    assert!((record["speed"]["value"].as_f64().unwrap() - 8.0 / 3.0).abs() < 1e-9);
    assert_eq!(record["speed"]["low"], 2.0);
    assert_eq!(record["speed"]["high"], 4.0);
    assert!((record["transcode_seconds"]["value"].as_f64().unwrap() - 1.5).abs() < 1e-9);
    assert_eq!(
        record["output_bytes"],
        serde_json::json!({"value": 42, "low": 42, "high": 42})
    );
    assert!((record["size_percent"].as_f64().unwrap() - 840.0).abs() < 1e-9);
    assert_eq!(record["quality"]["frames"], 2);
    assert_eq!(record["quality"]["source_stream_index"], 0);
    assert_eq!(record["quality"]["vmaf"]["value"], 96.5);
    assert_eq!(record["quality"]["ssim"]["value"], 0.99);
    assert_eq!(record["quality"]["psnr_y_db"]["value"], 42.0);
    assert_eq!(record["samples"].as_array().unwrap().len(), 2);
    assert_eq!(record["samples"][0]["start_seconds"], 0.0);
    assert_eq!(record["samples"][0]["timed_payload_bytes"], 10);
    assert_eq!(record["samples"][0]["vmaf"], 96.5);
    assert_eq!(record["samples"][0]["scored_frames"], 1);
    assert_eq!(record["samples"][1]["start_seconds"], 2.0);
    assert_eq!(record["result"]["message"], serde_json::Value::Null);
}
