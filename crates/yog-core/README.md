# yog-core

```rust,no_run
use std::{ffi::OsStr, time::Duration};
use yog_core::ffmpeg::Ffmpeg;

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), yog_core::error::Error> {
let ffmpeg = Ffmpeg::new("ffmpeg", Some(Duration::from_secs(3600)));
let result = ffmpeg.execute(
    ["-i", "input.mkv", "-map", "0", "-c", "copy", "output.mkv"].map(OsStr::new),
    |progress| println!("{:?}", progress.out_time_us),
    |stderr_bytes| { /* 实时处理原始诊断字节 */ },
).await?;
assert!(result.status.success());
# Ok(())
# }
```

```rust,no_run
use std::{path::Path, time::Duration};
use yog_core::ffprobe::Ffprobe;

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), yog_core::error::Error> {
let probe = Ffprobe::new("ffprobe", Some(Duration::from_secs(90)));
let input = Path::new("input.mkv");
let media = probe.probe(input).await?;
for stream in &media.output.streams {
    println!("{} {:?}", stream.index, stream.codec_name);
}

let packet_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
let count = packet_bytes.clone();
let result = probe.packets(input, None, "%+5", move |packet| {
    if let Some(size) = packet.size.as_deref().and_then(|size| size.parse::<u64>().ok()) {
        count.fetch_add(size, std::sync::atomic::Ordering::Relaxed);
    }
}).await?;
println!("sample bytes: {}, diagnostics: {:?}", packet_bytes.load(std::sync::atomic::Ordering::Relaxed), result.stderr);
# Ok(())
# }
```

```rust,no_run
use std::{path::Path, time::Duration};
use yog_core::{
    ffmpeg::{
        Ffmpeg,
        encoding::{Preset, RateControl},
        plan::{TranscodeRequest, VideoAction},
    },
    ffprobe::Ffprobe,
};

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> Result<(), yog_core::error::Error> {
let input = Path::new("input.mkv");
let media = Ffprobe::new("ffprobe", Some(Duration::from_secs(30))).probe(input).await?.output;
let plan = TranscodeRequest::mkv(input, "output.mkv")
    .with_video(VideoAction::encode_x264(
        Some(RateControl::Quality(23)), Some(Preset::Medium),
    ))
    .plan(&media);
Ffmpeg::new("ffmpeg", Some(Duration::from_secs(3600)))
    .execute(plan.args(), |_| {}, |_| {}).await?;
# Ok(())
# }
```

```rust,no_run
use yog_core::ffmpeg::{decoding::DecodingBackend, plan::{TranscodeRequest, VideoAction}};
let request = TranscodeRequest::mkv("input.mkv", "output.mkv")
    .with_decoding(DecodingBackend::Vaapi(Some("/dev/dri/renderD128".into())));
```
