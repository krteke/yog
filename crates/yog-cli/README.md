# yog CLI

```sh
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --encode-x264 --preset medium --quality 23
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --copy
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --decode-vaapi=/dev/dri/renderD128 --encode-nvenc h264
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --encode-x265 --preset slow --bitrate 4000000
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --decode-vaapi --encode-vaapi hevc --device /dev/dri/renderD128 --quality 27
cargo run -p yog-cli -- --help
cargo run -p yog-cli -- transcode --encode-nvenc --help
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --verify --encode-x264
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --vmaf --encode-x264
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --vmaf=5 --encode-x264
cargo run -p yog-cli -- predict -i input.mkv --encode-x264 --quality 23
cargo run -p yog-cli -- emulate -i input.mkv --png quality.png --svg quality.svg --range 18,30 --encode-x264 --preset medium
cargo run -p yog-cli -- transcode -i input.mp4 -o output.mp4 -C mp4 -v --copy
cargo run -p yog-cli -- transcode -i input.mkv -o output.mkv --quiet --copy
```

[config.toml](./config.example.toml)

```sh
cargo run -p yog-cli -- -i input.mkv transcode -o output.mkv --encode-x265 --quality 23 --vmaf --report run.jsonl
cargo run -p yog-cli -- -i media/ predict --recursive --encode-svt-av1 --quality 30 --report estimates.jsonl
```

### File format

JSONL

```json
{"transcode":{"input":"/media/in.mkv","output":"/media/out.mkv","status":"success","container":"mkv","video":{"action":"encode","encoder":"libx265","rate":{"kind":"quality","parameter":"CRF","value":23},"preset":"medium","multipass":null},"decoding":"software","source":{"bytes":734003200,"duration_seconds":3661.5,"streams":{"video":1,"audio":2,"subtitle":1,"attachment":1,"cover":0,"other":0},"video":{"codec":"hevc","width":1920,"height":1080,"frame_rate":"24000/1001","bit_rate":"1500000"},"audio":[{"codec":"aac","channels":2,"sample_rate":"48000"}]},"result":{"bytes":412345678,"duration_seconds":3661.5,"size_percent":56.18,"elapsed_seconds":812.4,"speed":4.507,"frames":87833,"fps":108.1},"verify":{"outcome":"complete","warnings":[]},"vmaf":{"n_subsample":7,"score":95.25,"error":null},"error":null}}
```

```json
{"predict":{"input":"/media/in.mkv","status":"success","container":"mkv","video":{"action":"encode","encoder":"libx264","rate":{"kind":"quality","parameter":"crf","value":23},"preset":null,"multipass":null},"decoding":"software","sampling":{"requested_samples":5,"sample_seconds":2.0,"measured_samples":2,"sampled_seconds":4.0},"source":{"bytes":734003200,"duration_seconds":3661.5,"streams":{"video":1,"audio":2,"subtitle":1,"attachment":1,"cover":0,"other":0},"video":{"codec":"h264","width":1920,"height":1080,"frame_rate":"24000/1001","bit_rate":"1500000"},"audio":[{"codec":"aac","channels":2,"sample_rate":"48000"}]},"speed":{"value":2.667,"low":2.0,"high":4.0},"transcode_seconds":{"value":1372.7,"low":915.4,"high":1830.8},"output_bytes":{"value":61712345670,"low":50000000000,"high":70000000000},"size_percent":840.0,"quality":{"frames":240,"source_stream_index":0,"vmaf":{"value":96.5,"low":96.5,"high":96.5},"ssim":{"value":0.99,"low":0.99,"high":0.99},"psnr_y_db":{"value":42.0,"low":42.0,"high":42.0}},"samples":[{"start_seconds":0.0,"duration_seconds":2.0,"encode_seconds":0.75,"speed":2.667,"timed_payload_bytes":3000000,"scored_frames":120,"vmaf":96.5,"ssim":0.99,"psnr_y_db":42.0}],"error":null}}
```

```json
{"skipped":{"input":"/media/notes.md","reason":"skipping because ffprobe failed: probe failed: ..."}}
```

### Conventions

- **Units.** Bytes are bytes, durations are seconds, `size_percent` is
  `output bytes / input bytes * 100` (it can exceed 100), `speed` is processed
  media seconds per wall-clock second, and `frame_rate` is ffprobe's
  `"numerator/denominator"` string.
- **Nulls.** Every field is always present. Unknown values are `null`, and
  optional objects such as `verify`, `vmaf`, `result`, and `source` are `null`
  as a whole. Array fields are `[]`.

### Field reference

`transcode` records:

| Field | Description |
| --- | --- |
| `input`, `output` | Absolute paths of the source and the published output. |
| `status` | `success`, `failure`, or `cancelled`. |
| `container` | Resolved output container extension (`mkv`, `mp4`, ...); `null` before the container is resolved. |
| `video.action` | `copy` or `encode`. |
| `video.encoder` | ffmpeg encoder name (`libx265`, `hevc_nvenc`, ...); `null` for `copy`. |
| `video.rate.kind` | `quality` or `bitrate`. |
| `video.rate.parameter` | ffmpeg option carrying the rate: `CRF`, `CQ`, `QP`, `global_quality`, or `b`. |
| `video.rate.value` | Quality value or bitrate in bits per second. |
| `video.preset` | Encoder speed/effort knob: a string for x264/x265/NVENC/QSV (`medium`, `p4`, ...) and a number for SVT-AV1/AOM AV1/rav1e (`cpu-used`/`speed`). |
| `video.multipass` | NVENC multipass mode (`disabled`, `qres`, `fullres`), otherwise `null`. |
| `decoding` | `software`, or `vaapi`, `cuda`, `qsv` with an optional `:<device>`. |
| `source` | Probed input facts: `bytes`, `duration_seconds`, stream counts, the first regular video stream, and every audio stream. `streams.cover` counts cover art (`attached_pic`), which is excluded from `streams.video`. |
| `source.video.bit_rate` | Input video bit rate in bits per second. |
| `result` | Measured output: published `bytes`, `duration_seconds` from ffmpeg's final progress report, `size_percent`, `elapsed_seconds` for the ffmpeg run, aggregate `speed`, last reported `frames` and `fps`. |
| `verify` | Present only with `--verify`: `outcome` is `complete`, `missing_audio_after_seek`, or `incomplete`, plus the warning strings printed to stderr. |
| `vmaf` | Present only with `--vmaf`: the requested `n_subsample`, the `score` or the `error`. |
| `error` | `null` on success, otherwise the failure chain as a single string. |

`predict` records replace `result`/`verify`/`vmaf` with the prediction itself:

| Field | Description |
| --- | --- |
| `sampling` | The prediction settings from the config: `requested_samples`, `sample_seconds`, and the `measured_samples`/`sampled_seconds` that were actually encoded. |
| `speed`, `transcode_seconds`, `output_bytes` | `{value, low, high}` estimates. |
| `size_percent` | Estimated output size as a percentage of the input size. |
| `quality` | Aggregated `vmaf`/`ssim`/`psnr_y_db` estimates over all scored `frames`, and the `source_stream_index` they were scored against. |
| `samples` | Per-sample measurements: window `start_seconds` and `duration_seconds`, `encode_seconds`, `speed`, `timed_payload_bytes`, `scored_frames`, and the quality metrics. |

`skipped` records describe inputs that were discovered but never processed
because `ffprobe` failed; files that probe successfully but contain no video
stream are ignored. Every other line is `transcode` or `predict`.
