# yog-core

```rust,no_run
use std::{ffi::OsStr, time::Duration};
use yog_core::ffmpeg::Ffmpeg;

let ffmpeg = Ffmpeg::new("ffmpeg", Duration::from_secs(3600));
let result = ffmpeg.execute(
    ["-i", "input.mkv", "-map", "0", "-c", "copy", "output.mkv"].map(OsStr::new),
    |progress| println!("{:?}", progress.out_time_us),
    |stderr_bytes| { /* 实时处理原始诊断字节 */ },
)?;
assert!(result.status.success());
# Ok::<(), yog_core::error::Error>(())
```

```rust,no_run
use std::{path::Path, time::Duration};
use yog_core::ffprobe::Ffprobe;

let probe = Ffprobe::new("ffprobe", Duration::from_secs(90));
let input = Path::new("input.mkv");
let media = probe.probe(input)?;
for stream in &media.output.streams {
    println!("{} {:?}", stream.index, stream.codec_name);
}

let mut packet_bytes = 0_u64;
let result = probe.packets(input, None, "%+5", |packet| {
    if let Some(size) = packet.size.as_deref().and_then(|size| size.parse::<u64>().ok()) {
        packet_bytes += size;
    }
})?;
println!("sample bytes: {packet_bytes}, diagnostics: {:?}", result.stderr);
# Ok::<(), yog_core::error::Error>(())
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

let input = Path::new("input.mkv");
let media = Ffprobe::new("ffprobe", Duration::from_secs(30)).probe(input)?.output;
let plan = TranscodeRequest::mkv(input, "output.mkv")
    .with_video(VideoAction::encode_x264(
        Some(RateControl::Quality(23)), Some(Preset::Medium),
    ))
    .plan(&media);
Ffmpeg::new("ffmpeg", Duration::from_secs(3600))
    .execute(plan.args(), |_| {}, |_| {})?;
# Ok::<(), yog_core::error::Error>(())
```
