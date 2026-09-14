# yog-core

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
    if let Ok(size) = packet.try_size() {
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
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let input = Path::new("input.mkv");
let ffmpeg = Ffmpeg::new("ffmpeg", Some(Duration::from_secs(3600)));
let media = Ffprobe::new("ffprobe", Some(Duration::from_secs(30))).probe(input).await?.output;
let plan = TranscodeRequest::mkv(input, "output.mkv")
    .with_video(VideoAction::encode_x264(
        Some(RateControl::Quality(23)), Some(Preset::Medium),
    ))
    .plan(&media, &ffmpeg).await?;
let command = ffmpeg.build(&plan)?;
command.run(|_| {}, |_| {}).await?;
# Ok(())
# }
```

```rust,no_run
use yog_core::ffmpeg::{decoding::DecodingBackend, plan::{TranscodeRequest, VideoAction}};
let request = TranscodeRequest::mkv("input.mkv", "output.mkv")
    .with_decoding(DecodingBackend::Vaapi(Some("/dev/dri/renderD128".into())));
```
