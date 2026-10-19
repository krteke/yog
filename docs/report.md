# Report JSONL reference

## File format and write timing

Enable reporting with `--report PATH`. The file uses UTF-8 JSONL: each line is a complete JSON object terminated by a newline.

By default, only new files are created. An existing file requires `--overwrite-report` to be truncated and rewritten. Records are written and flushed individually during a run; records already written are retained after cancellation or failure. Failure to create the file prevents the run from starting. Write or final flush failures during execution produce diagnostic warnings without rolling back published videos.

The outermost object has exactly one type key: `transcode`, `predict`, `emulate`, or `skipped`. For example:

```json
{"skipped":{"input":"/media/input/movie.mkv","result":{"status":"skipped","reason":"existing_output","message":"output file already exists: /media/output/movie.mkv"}}}
```

Batch skips due to probing or eligibility checks, State reuse, and manual skips of tasks not yet started use `skipped`. Tasks that have already entered the transcoding workflow may also record a manual skip in `transcode.result`.

Transcode / Predict typically write one line per processed item. Emulate writes one line per attempted quality point.

## Common conventions

Byte fields use raw bytes, durations use seconds, bitrates use bit/s, and speed means "media seconds / elapsed wall-clock seconds".

Unknown or not-yet-measured optional fields are usually `null`; empty arrays are `[]`. Unused `skipped.recovery` and `vmaf.skip_reason` fields are omitted. Failed records may retain source information obtained before the failure.

### `result`: result classification

Success: `{"status":"success"}`.

Skipped: `{"status":"skipped","reason":"no_video","message":"no regular video streams"}`.

Failed: `{"status":"failed","phase":"transcoding","timed_out":false,"message":"transcode failed: ..."}`.

Cancelled: `{"status":"cancelled"}`.

The meanings of `reason` values are:

- `manual`: the user manually skipped the item, or a previous manual skip was retained.
- `existing_output`: batch processing found an existing final output file without overwrite permission.
- `already_completed`: a successful State record was reused, either through a normal match or by explicitly trusting it with Force.
- `resume_mismatch`: recovery found differences, and the user or a preset policy selected Skip.
- `no_video`: no regular video streams.
- `probe_failed`: FFprobe could not probe the batch input.
- `probe_timed_out`: FFprobe timed out.
- `unsupported_container`: a supported output container could not be selected; an explicit container is usually needed.

`phase` uses these snake_case strings: `discovering`, `probing`, `preparing_output`, `planning`, `predicting`, `transcoding`, `verifying`, `publishing`, `calculating_vmaf`, and `rendering_chart`.

`timed_out` provides a structured timeout classification.

### `video`: requested encoding settings

- `action`: `copy` or `encode`.
- `encoder`: the FFmpeg encoder name, such as `libx265` or `hevc_nvenc`; `null` for copying.
- `rate`: `null`, or an object such as `{ "kind": "quality", "parameter": "CRF", "value": 23 }`. `kind` is `quality / bitrate`. Quality parameters are `CRF / CQ / QP / global_quality`; the bitrate parameter is `b`. `value` is a quality level or a bitrate in bit/s.
- `preset`: a string or `null`. Named presets include `medium` and `p5`. Numeric SVT-AV1 / AOM / rav1e settings are also written as strings, such as `"6"`, representing preset / cpu-used / speed respectively.
- `multipass`: NVENC's `disabled / qres / fullres`, otherwise `null`.
- `device`: the display path of the VAAPI encoding device, otherwise `null`.

A `null` rate or preset means it was not explicitly specified. For copying, all other encoding fields are `null`.

`decoding` is a display string: `software`, `vaapi`, `cuda`, or `qsv`. With a device, it looks like `vaapi:/dev/dri/renderD128`.

`container` is the resolved container extension, such as `mkv`. It may be `null` if failure occurs before resolution.

### `source`: source media facts

`source` is an object or `null`, containing:

- `bytes`: source file size, or `null` if unavailable.
- `duration_seconds`: a valid positive media duration, or `null` if unknown.
- `streams`: counts for `video / audio / subtitle / attachment / cover / other`. `cover` counts attached pictures, which are not also counted as `video`.
- `video`: the first regular video stream, or `null`. Fields are `codec`, `width`, `height`, `frame_rate`, and `bit_rate`. The last two retain FFprobe strings, such as `"24000/1001"` and `"1500000"`.
- `audio`: an array of all audio streams. Each item has `codec`, `channels`, and `sample_rate`; the sample rate retains its string representation, such as `"48000"`.

## `transcode` records

Contain `input`, `output`, `result`, `container`, `video`, `decoding`, and `source`, plus the following three groups of optional information.

### `metrics`

Measurements obtained after successful publication:

- `bytes`: final file size.
- `duration_seconds`: the output duration reported by FFmpeg's last progress update, or `null` if absent.
- `size_percent`: output bytes / source bytes × 100. It can exceed 100, and is `null` when valid sizes are unavailable.
- `elapsed_seconds`: actual FFmpeg transcoding runtime, excluding time spent on batch scanning, initial probing, subsequent verification, and VMAF.
- `speed`: processed duration / FFmpeg transcoding runtime. The progress-reported duration is preferred; source media duration may be used when it is unavailable.
- `frames`, `fps`: frame count and fps from the last progress update, or `null` if absent.

Results that were not successfully published usually have no `metrics`.

### `verify`

`null` when verification was not requested. Otherwise, it contains `outcome` and `warnings`:

- `complete`: verification completed.
- `missing_audio_after_seek`: verification detected missing audio, a definite verification failure.
- `incomplete`: verification could not be completed. Reasons are recorded in `warnings`; this does not necessarily fail the transcode.

Use `result` to determine the final outcome.

### `vmaf`

`null` when scoring was not requested. Otherwise, it contains `n_subsample`, `score`, and `error`, and may include `skip_reason`. `n_subsample: null` means every frame; N means sampling every N frames.

On success, `score` has a value. On failure or cancellation, `error` contains the explanation. If the user skips only the scoring phase, `skip_reason` is `manual`.

## `predict` records

Contain the shared `input / result / container / video / decoding / source` fields, with no video `output`, plus:

- `sampling.requested_samples`, `sample_seconds`: requested sample count and duration per sample.
- `sampling.measured_samples`, `sampled_seconds`: actual measured sample count and total sampled duration.
- `speed`: estimated speed.
- `transcode_seconds`: estimated full-video transcoding time in seconds.
- `output_bytes`: estimated full-video output size in bytes.
- `size_percent`: central size estimate / source size × 100.
- `quality`: aggregated sample quality, containing `frames` (scored frame count), `source_stream_index` (the source stream index used for scoring), `vmaf`, `ssim`, and `psnr_y_db`.
- `samples`: an array of raw sample measurements.

Estimates use `{ "value": 96.5, "low": 95.0, "high": 97.2 }` for the central estimate and lower / upper bounds. These are not full-video measurements or guarantees at a fixed confidence level. SSIM / PSNR-Y are `null` when not calculated or unavailable.

Each sample has `start_seconds`, `duration_seconds`, `encode_seconds`, `speed`, `timed_payload_bytes`, `scored_frames`, `vmaf`, `ssim`, and `psnr_y_db`.

Failed predictions retain request information. All estimate objects may be `null`, `samples` may be `[]`, and the actual sample count may be zero.

## `emulate` records

Flatten all Predict fields into the same object, with these additional fields:

- `candidate_index`: the one-based Candidate index, or `null` when using a single encoder subcommand.
- `outputs`: requested absolute paths for `png` and `svg`; an unrequested format is `null`.

`video.rate` records the quality point actually attempted, so results can be grouped by `candidate_index + video.rate.value`. Each configuration's points are written in ascending quality order. On cancellation, only attempted points are retained. Failed points also have their own `result`, and successful points can still generate charts.

## `skipped` and recovery information

A minimal record contains only `input` and `result`. State reuse and recovery-policy skips additionally include:

```json
{"recovery":{"forced":false,"output":"/media/output/movie.mkv"}}
```

This is a fragment inside a record, not a standalone Report line. `forced` indicates that the user ignored recovery differences and trusted the record. `output` is the output location supplied by the recovery record, and may be `null`. Not every skip has this object.

## Example: streaming statistics

```python
import collections
import json

counts = collections.Counter()
with open("run.jsonl", encoding="utf-8") as report:
    for line in report:
        kind, record = next(iter(json.loads(line).items()))
        result = record["result"]
        counts[(kind, result["status"], result.get("reason"))] += 1
print(counts)
```
