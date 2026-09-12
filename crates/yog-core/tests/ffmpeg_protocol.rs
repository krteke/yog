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

#[tokio::test]
#[ignore = "requires installed ffmpeg and ffprobe"]
async fn generated_media_exercises_progress_metadata_frames_and_packets() {
    let scratch = Scratch::new();
    let input = scratch.0.join("-媒体 with spaces.mkv");
    let encoded = Command::new("ffmpeg")
        .args(
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
        )
        .output()
        .unwrap();
    assert!(encoded.status.success());

    let probe = Ffprobe::new("ffprobe", Some(TIMEOUT));
    let mut media = probe.probe(&input).await.unwrap().output;
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
    // Verify FFmpeg follows the caller's stream order.
    media.streams.reverse();
    let request = TranscodeRequest::mkv(&input, &destination).with_video(VideoAction::encode_x264(
        Some(RateControl::Quality(23)),
        Some(Preset::Ultrafast),
    ));
    let ffmpeg = Ffmpeg::new("ffmpeg", Some(TIMEOUT));
    let plan = request.plan(&media, &ffmpeg).await.unwrap();
    let mut progress = Vec::new();
    ffmpeg
        .build(&plan)
        .unwrap()
        .run(|record| progress.push(record), |_| {})
        .await
        .unwrap();
    let last = progress.last().unwrap();
    assert!(last.finished);
    assert_eq!(last.frame, Some(3));
    assert!(last.out_time_us.unwrap() > 0);
    assert!(last.speed.unwrap() > 0.0);
    assert!(last.total_size.unwrap() > 0);
    let actual = probe.probe(&destination).await.unwrap().output;
    assert_eq!(actual.streams.len(), 2);
    assert_eq!(actual.streams[0].codec_name, media.streams[0].codec_name);
    assert_eq!(actual.streams[1].codec_name.as_deref(), Some("h264"));
    assert_eq!(actual.format.tags["title"], "protocol fixture");
    let before = fs::read(&destination).unwrap();
    // Some FFmpeg builds report exit 0 when -n refuses an existing output.
    // The planner's overwrite contract is that the file stays untouched.
    let _existing = ffmpeg.build(&plan).unwrap().run(|_| {}, |_| {}).await;
    assert_eq!(fs::read(&destination).unwrap(), before);
    let copied = scratch.0.join("remux.mkv");
    let remux = TranscodeRequest::mkv(destination, &copied)
        .plan(&actual, &ffmpeg)
        .await
        .unwrap();
    ffmpeg
        .build(&remux)
        .unwrap()
        .run(|_| {}, |_| {})
        .await
        .unwrap();
    let copied = probe.probe(&copied).await.unwrap().output;
    assert_eq!(copied.streams[0].codec_name, actual.streams[0].codec_name);
    assert_eq!(copied.streams[1].codec_name, actual.streams[1].codec_name);

    let (sender, receiver) = std::sync::mpsc::channel();
    probe
        .frames(&input, 0, "%+1", move |frame| sender.send(frame).unwrap())
        .await
        .unwrap();
    let frames: Vec<_> = receiver.try_iter().collect();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].pix_fmt.as_deref(), Some("yuv420p"));
    let (sender, receiver) = std::sync::mpsc::channel();
    probe
        .packets(&input, None, "%+1", move |packet| {
            sender.send(packet).unwrap()
        })
        .await
        .unwrap();
    let packets: Vec<_> = receiver.try_iter().collect();
    assert!(packets.iter().any(|packet| packet.stream_index == 1));
    assert!(
        packets
            .iter()
            .all(|packet| packet.size.as_ref().unwrap().parse::<u64>().unwrap() > 0)
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    probe
        .packets(&input, Some(0), "%+1", move |packet| {
            sender.send(packet).unwrap()
        })
        .await
        .unwrap();
    let video_packets: Vec<_> = receiver.try_iter().collect();
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
    let audio = probe.probe(&audio).await.unwrap().output;
    assert_eq!(audio.streams.len(), 1);
    assert_eq!(audio.streams[0].codec_type.as_deref(), Some("audio"));

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let byte_path = scratch
            .0
            .join(std::ffi::OsString::from_vec(b"non-utf8-\xff.mkv".to_vec()));
        fs::copy(&input, &byte_path).unwrap();
        assert_eq!(
            probe.probe(&byte_path).await.unwrap().output.streams.len(),
            2
        );
    }

    let failure = probe
        .probe(&scratch.0.join("missing.mkv"))
        .await
        .unwrap_err();
    assert!(!failure.stderr.is_empty());
    assert!(!failure.status.unwrap().success());
    assert!(matches!(failure.reason, Failure::Probe(_)));
}

#[tokio::test]
#[ignore = "requires installed ffmpeg and ffprobe"]
async fn shared_video_encoding_keeps_reordered_cover_and_audio_as_copy() {
    use yog_core::ffmpeg::{
        encoding::{Preset, RateControl},
        plan::{TranscodeRequest, VideoAction},
    };

    let scratch = Scratch::new();
    let input = scratch.0.join("with-cover.mp4");
    let output = scratch.0.join("encoded.mp4");
    let ffmpeg = Ffmpeg::new("ffmpeg", Some(TIMEOUT));
    let generated = Command::new("ffmpeg")
        .args(
            [
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=10:duration=0.3",
                "-f",
                "lavfi",
                "-i",
                "color=blue:size=64x64:rate=10:duration=0.3",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.3",
                "-f",
                "lavfi",
                "-i",
                "color=red:size=64x64:rate=10:duration=0.1",
                "-map",
                "0:v",
                "-map",
                "2:a",
                "-map",
                "1:v",
                "-map",
                "3:v",
                "-c:v",
                "mpeg4",
                "-c:v:2",
                "mjpeg",
                "-disposition:v:2",
                "attached_pic",
                "-c:a",
                "aac",
            ]
            .into_iter()
            .map(std::ffi::OsString::from)
            .chain([input.clone().into_os_string()]),
        )
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let probe = Ffprobe::new("ffprobe", Some(TIMEOUT));
    let mut media = probe.probe(&input).await.unwrap().output;
    assert_eq!(media.streams.len(), 4);
    let cover_position = media
        .streams
        .iter()
        .position(|stream| stream.disposition.get("attached_pic") == Some(&1))
        .unwrap();
    let cover = media.streams.remove(cover_position);
    assert_ne!(cover.index, 0);
    media.streams.insert(0, cover);
    // The copy override must use output index 0, not the cover's input index.
    let plan = TranscodeRequest::mp4(&input, &output)
        .with_video(VideoAction::encode_x264(
            Some(RateControl::Quality(23)),
            Some(Preset::Ultrafast),
        ))
        .plan(&media, &ffmpeg)
        .await
        .unwrap();
    ffmpeg
        .build(&plan)
        .unwrap()
        .run(|_| {}, |_| {})
        .await
        .unwrap();
    let actual = probe.probe(&output).await.unwrap().output;
    assert_eq!(actual.streams.len(), 4);
    assert_eq!(
        actual
            .streams
            .iter()
            .filter(|stream| stream.codec_name.as_deref() == Some("h264"))
            .count(),
        2
    );
    let cover = actual
        .streams
        .iter()
        .find(|stream| stream.disposition.get("attached_pic") == Some(&1))
        .unwrap();
    assert_eq!(cover.codec_name.as_deref(), Some("mjpeg"));

    // Compare copied packet payloads, not merely codecs that could also be re-encoded.
    for selector in ["0:disp:attached_pic", "0:a"] {
        let mut payloads = Vec::new();
        for path in [&input, &output] {
            let result = Command::new("ffmpeg")
                .args(["-v", "error", "-nostdin", "-i"])
                .arg(path)
                .args(["-map", selector, "-c", "copy", "-f", "data", "pipe:1"])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(!result.stdout.is_empty());
            payloads.push(result.stdout);
        }
        assert_eq!(payloads[0], payloads[1], "{selector}");
    }
}

#[tokio::test]
#[ignore = "requires installed ffmpeg and ffprobe"]
async fn installed_tools_exercise_capability_parsers() {
    let ffmpeg = Ffmpeg::new("ffmpeg", Some(TIMEOUT));
    let help = ffmpeg.encoder_help("ffv1").await.unwrap();
    assert!(
        help.pixel_formats
            .unwrap()
            .iter()
            .any(|format| format == "yuv420p")
    );
    assert!(
        ffmpeg
            .encoder_help("yog_nonexistent_encoder")
            .await
            .is_err()
    );
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
