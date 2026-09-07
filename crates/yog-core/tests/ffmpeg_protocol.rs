use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use yog_core::error::Failure;
use yog_core::ffmpeg::Ffmpeg;
use yog_core::ffprobe::Ffprobe;

const TIMEOUT: Duration = Duration::from_secs(10);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yog-protocol-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "requires installed ffmpeg and ffprobe"]
fn generated_media_exercises_progress_metadata_frames_and_packets() {
    let scratch = Scratch::new();
    let input = scratch.0.join("-媒体 with spaces.mkv");
    let mut progress = Vec::new();
    let encoded = Ffmpeg::new("ffmpeg", TIMEOUT)
        .execute(
            [
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=16x16:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=8000",
                "-t",
                "0.3",
                "-c:v",
                "ffv1",
                "-c:a",
                "pcm_s16le",
                "-metadata",
                "title=protocol fixture",
            ]
            .into_iter()
            .map(std::ffi::OsString::from)
            .chain([input.clone().into_os_string()]),
            |record| progress.push(record),
            |_| {},
        )
        .unwrap();
    assert!(encoded.status.success());
    let last = progress.last().unwrap();
    assert!(last.finished);
    assert_eq!(last.frame, Some(3));
    assert!(last.out_time_us.unwrap() > 0);
    assert!(last.total_size.unwrap() > 0);

    let probe = Ffprobe::new("ffprobe", TIMEOUT);
    let media = probe.probe(&input).unwrap().output;
    assert_eq!(media.streams.len(), 2);
    assert_eq!(media.streams[0].codec_name.as_deref(), Some("ffv1"));
    assert_eq!(media.streams[1].codec_type.as_deref(), Some("audio"));
    assert_eq!(media.format.tags["title"], "protocol fixture");
    // Exercise the complete probe -> plan -> execute -> probe path.
    use yog_core::ffmpeg::{
        encoding::{Preset, RateControl},
        plan::{TranscodeRequest, VideoAction},
    };
    let destination = scratch.0.join("planned 输出.mkv");
    let request = TranscodeRequest::mkv(&input, &destination).with_video(VideoAction::encode_x264(
        Some(RateControl::Quality(23)),
        Some(Preset::Ultrafast),
    ));
    let plan = request.plan(&media);
    let ffmpeg = Ffmpeg::new("ffmpeg", TIMEOUT);
    let mut completed = false;
    ffmpeg
        .execute(plan.args(), |record| completed = record.finished, |_| {})
        .unwrap();
    assert!(completed);
    let actual = probe.probe(&destination).unwrap().output;
    assert_eq!(actual.streams.len(), 2);
    assert_eq!(actual.streams[0].codec_name.as_deref(), Some("h264"));
    assert_eq!(actual.streams[1].codec_name, media.streams[1].codec_name);
    assert_eq!(actual.format.tags["title"], "protocol fixture");
    let before = fs::read(&destination).unwrap();
    // Some FFmpeg builds report exit 0 when -n refuses an existing output.
    // The planner's overwrite contract is that the file stays untouched.
    let _existing = ffmpeg.execute(plan.args(), |_| {}, |_| {});
    assert_eq!(fs::read(&destination).unwrap(), before);
    let copied = scratch.0.join("remux.mkv");
    let remux = TranscodeRequest::mkv(destination, &copied).plan(&actual);
    ffmpeg.execute(remux.args(), |_| {}, |_| {}).unwrap();
    let copied = probe.probe(&copied).unwrap().output;
    assert_eq!(copied.streams[0].codec_name, actual.streams[0].codec_name);
    assert_eq!(copied.streams[1].codec_name, actual.streams[1].codec_name);

    let mut frames = Vec::new();
    probe
        .frames(&input, 0, "%+1", |frame| frames.push(frame))
        .unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].pix_fmt.as_deref(), Some("yuv420p"));
    let mut packets = Vec::new();
    probe
        .packets(&input, None, "%+1", |packet| packets.push(packet))
        .unwrap();
    assert!(packets.iter().any(|packet| packet.stream_index == 1));
    assert!(
        packets
            .iter()
            .all(|packet| packet.size.as_ref().unwrap().parse::<u64>().unwrap() > 0)
    );
    let mut video_packets = Vec::new();
    probe
        .packets(&input, Some(0), "%+1", |packet| video_packets.push(packet))
        .unwrap();
    assert_eq!(video_packets.len(), 3);
    assert!(video_packets.iter().all(|packet| packet.stream_index == 0));

    // A probe adapter must accept audio-only input; the old workflow rejected it.
    let audio = scratch.0.join("audio.wav");
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(&input)
        .args(["-vn", "-c:a", "copy"])
        .arg(&audio)
        .output()
        .unwrap();
    assert!(output.status.success());
    let audio = probe.probe(&audio).unwrap().output;
    assert_eq!(audio.streams.len(), 1);
    assert_eq!(audio.streams[0].codec_type.as_deref(), Some("audio"));

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let byte_path = scratch
            .0
            .join(std::ffi::OsString::from_vec(b"non-utf8-\xff.mkv".to_vec()));
        fs::copy(&input, &byte_path).unwrap();
        assert_eq!(probe.probe(&byte_path).unwrap().output.streams.len(), 2);
    }

    let failure = probe.probe(&scratch.0.join("missing.mkv")).unwrap_err();
    assert!(!failure.stderr.is_empty());
    assert!(!failure.status.unwrap().success());
    assert!(matches!(failure.reason, Failure::Probe(_)));
}

#[test]
#[ignore = "requires installed ffmpeg and ffprobe"]
fn installed_tools_exercise_capability_parsers() {
    let ffmpeg = Ffmpeg::new("ffmpeg", TIMEOUT);
    let help = ffmpeg.encoder_help("ffv1").unwrap();
    assert!(
        help.pixel_formats
            .unwrap()
            .iter()
            .any(|format| format == "yuv420p")
    );
    assert!(ffmpeg.encoder_help("yog_nonexistent_encoder").is_err());
}

#[test]
#[ignore = "requires installed ffmpeg with libvmaf"]
fn actual_libvmaf_json_decodes_typed_frame_metrics() {
    let scratch = Scratch::new();
    let output = Command::new("ffmpeg").current_dir(&scratch.0).args([
        "-v", "error", "-nostdin", "-f", "lavfi", "-i",
        "testsrc2=size=192x108:rate=5:duration=0.4", "-filter_complex",
        "[0:v]split=2[a][b];[a][b]libvmaf=feature=name=psnr|name=float_ssim:log_fmt=json:log_path=metrics.json",
        "-f", "null", "-",
    ]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log: yog_core::ffmpeg::vmaf::VmafLog =
        serde_json::from_slice(&fs::read(scratch.0.join("metrics.json")).unwrap()).unwrap();
    assert_eq!(log.frames.len(), 2);
    assert_eq!(log.frames[1].frame_num, 1);
    assert!(
        log.frames
            .iter()
            .all(|frame| frame.metrics.vmaf.unwrap() > 90.0)
    );
    assert!(log.frames[0].metrics.float_ssim.unwrap() > 0.99);
    assert!(log.frames[0].metrics.psnr_y.unwrap() > 40.0);
}
