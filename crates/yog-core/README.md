# yog-core

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
