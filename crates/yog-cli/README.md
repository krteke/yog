# Yog CLI

Yog processes video through FFmpeg / FFprobe and provides three subcommands: `transcode` performs actual transcoding, `predict` makes sample-based predictions for one configuration, and `emulate` predicts multiple quality levels or configurations and plots curves.

## Running the CLI

```sh
cargo run -p yog-cli -- --help
```

View help for an operation or encoder:

```sh
yog transcode --help
yog transcode --encode-nvenc --help
```

The first command shows transcoding options; the second shows NVENC encoding options. Encoders can also be selected by their regular subcommand names, such as `x264`. This guide consistently uses the `--encode-x264` form.

FFmpeg and FFprobe must be available before running tasks. Your local FFmpeg build and drivers must support the selected encoders, hardware backends, and filters. Predict, Emulate, and post-transcode VMAF calculations require `libvmaf`.

## Common options

These options apply to all three subcommands. They can appear before or after the operation name, or after the encoder subcommand.

### Input: `-i / --input PATH`

An input is required for every run. Pass a file in normal mode, or a directory for `transcode --recursive` and `predict --recursive`.

### Container: `-C / --container FORMAT`

Explicitly select the container format. Supported values are `mkv`, `mp4`, `mov`, `m4a`, `3gp`, `3g2`, `f4v`, `ismv`, `psp`, `webm`, `ts`, `m2ts`, `avi`, `flv`, `asf`, `wmv`, `mpg` (alias: `mpeg`), `vob`, `ogg`, and `ogv`.

When omitted, the container is selected from the input file and FFprobe format information. In single-file mode, keep the output extension consistent with the container. In batch mode, explicitly selecting a container replaces the output extensions accordingly.

Predict / Emulate use this option to select the sample output container. Container overhead is also part of the prediction conditions.

### Decoder

- No hardware decoding option: software decoding.
- `--decode-vaapi[=DEVICE]`: VAAPI decoding, for example `--decode-vaapi=/dev/dri/renderD128`.
- `--decode-cuda[=DEVICE]`: CUDA decoding, for example `--decode-cuda=0`.
- `--decode-qsv[=DEVICE]`: QSV decoding, with an optional device.

The three hardware decoding options are mutually exclusive. The device is optional; when supplied, it must be joined with `=`. Decoders and encoders are selected independently: VAAPI decoding can be paired with x265 software encoding.

### Configuration file: `-c / --config PATH`

Read a TOML configuration file. When omitted, Yog looks for `yog/config.toml` under the system configuration directory. On Linux, this is usually `$XDG_CONFIG_HOME/yog/config.toml`, or `~/.config/yog/config.toml` when that variable is unset. A missing automatically discovered file falls back to defaults; a missing explicitly specified file is an error.

[Configuration example](config.example.toml)

`progress_tick_interval_ms` controls the terminal progress refresh interval in milliseconds. `prediction.samples` is the requested sample count, and `sample_sec` is the requested duration of each sample in seconds. Predict and Emulate share these settings; the actual samples depend on conditions such as media length. `emulation.width / height` control chart dimensions in pixels. Omitted settings use defaults; unknown settings are rejected.

### Tools, timeout, and output

- `--ffmpeg PATH`, `--ffprobe PATH`: executable paths; by default, `ffmpeg` and `ffprobe` are located through PATH.
- `-t / --timeout SECONDS`: the execution time limit for each subprocess; unlimited by default.
- `--verbose`: detailed diagnostics and full prediction information, including ranges. The default log level is raised to Debug, showing actual executed commands and FFmpeg logs. Normal mode shows progress, a Predict spinner, and concise summaries.
- `--quiet`: disable runtime terminal output and logs; mutually exclusive with `--verbose`. Tasks still run, requested Report / State files are written, and exit codes are returned. Clap argument parsing errors are still displayed.
- `-h / --help`: help for the current command level. At the top level, `-V / --version` displays the version.

Logs can be filtered with `RUST_LOG`, for example `RUST_LOG=yog_core=debug yog ...`. This does not re-enable logs when `--quiet` is used.

## Transcode: actual transcoding

```sh
yog -i input.mkv transcode -o output.mkv --encode-x265 --quality 23 --preset medium
```

Encode video with x265 at CRF 23 and the medium preset. If no video action is selected, the default is `--copy`, which copies video streams. Audio, subtitles, attachments, and other streams are mapped and copied by the transcode plan according to container capabilities.

### Output and batch processing

`-o / --output PATH` is required. In single-file mode it is the final output path; with `-r / --recursive`, it is the output root directory. Batch processing preserves the relative input directory structure. Filenames are also preserved when no container is specified; selecting a container replaces their extensions.

Batch processing scans paths and checks output conflicts first, then probes and executes each file individually. Inputs without video, inputs that cannot be probed, and inputs for which no container can be selected are recorded as skipped. Video transcoding failures are recorded without stopping subsequent tasks, and a summary is printed at the end. Batch preparation errors, such as output mapping conflicts or failure to prepare the Report, still prevent execution from starting. Manual cancellation stops subsequent tasks and produces a summary.

Without overwrite permission, an existing final output file is recorded as **Skipped · Existing output**, and batch processing continues with the next item. Single-file mode still treats an existing output as an error.

`--overwrite-output` allows final video files to be replaced. Output is first written to `.part.filename` in the same directory, for example `.part.movie.mkv`, and is published as `movie.mkv` only after success. Existing `.part.*` files not claimed by recovery are not overwritten directly.

### Encoders and rate control

Encoder options also apply to Predict. Emulate uses encoder and preset settings, but determines quality from its quality points.

`-q / --quality N` selects the encoder's quality control. `-b / --bitrate N` specifies a positive integer video bitrate in **bit/s**, for example `4000000`. These options are mutually exclusive. When neither is supplied, FFmpeg / encoder defaults are used.

- `--copy`: Transcode only; copy video.
- `--encode-x264`, `--encode-x265`: `libx264`, `libx265`, with CRF in `0–51`. `-p / --preset` accepts `ultrafast / superfast / veryfast / faster / fast / medium / slow / slower / veryslow`.
- `--encode-svt-av1`: `libsvtav1`, with CRF in `0–63` and `-p / --preset` in `0–13`.
- `--encode-aom-av1`: `libaom-av1`, with CRF in `0–63`. `-u / --cpu-used` ranges from `0–8` and controls speed versus encoding efficiency.
- `--encode-rav1e`: `librav1e`, with QP in `0–255` and `-s / --speed` in `0–10`.
- `--encode-nvenc CODEC`: requires `h264 / hevc / av1`. CQ ranges from `0–51` for H.264 and HEVC, or `0–63` for AV1. `-p / --preset` accepts `p1–p7`. `-m / --multipass` accepts `disabled / qres / fullres`, meaning disabled, quarter-resolution analysis, and full-resolution analysis respectively.
- `--encode-qsv CODEC`: requires `h264 / hevc / av1`. `global_quality` ranges from `1–51`. `-p / --preset` accepts `veryfast / faster / fast / medium / slow / slower / veryslow`.
- `--encode-vaapi CODEC`: requires `h264 / hevc / av1`. H.264 and HEVC use QP in `0–52`; AV1 uses `global_quality` in `0–255`. `-d / --device PATH` selects the encoding device, defaulting to `/dev/dri/renderD128`. There is no preset option.

### Verification and VMAF

`-v / --verify` checks transcoding results before publication, including streams and relevant media checks. Copy mode also uses packet information for comparison. A definite verification failure fails the item and preserves any existing target; incomplete verification may only produce warnings. The outcome is written to the Report's `verify` field.

`--vmaf` or `--vmaf=full` calculates VMAF for every frame. `--vmaf=N` samples every N frames, where N is a positive integer, for example `--vmaf=5`. The optional value must be supplied using `=`.

VMAF runs independently after the video has been successfully published. The VMAF thread count is set from the available CPU parallelism; there is currently no separate CLI thread option.

### Report

`--report PATH` writes JSONL results containing statuses, encoding settings, source media facts, and available measurements. `--overwrite-report` allows an existing Report to be truncated and rewritten; without permission, an existing file prevents the run from starting.

### State and resuming batches

The following options are available only with `transcode --recursive`:

- By default, Yog reads `.yog-state.jsonl` from the output directory and creates it if missing. Matching completed records appear as **Already completed** and are not transcoded again. This differs from **Existing output**, which is based only on file existence.
- `--state PATH`: specify the state file.
- `--no-state`: neither read nor write State; mutually exclusive with the other recovery options. Existing outputs are still skipped.
- `--restart`: discard previous completion / skip decisions and reschedule using the current settings. This does not automatically grant output overwrite permission. To actually re-encode existing outputs, also pass `--overwrite-output`.
- `--retry-manual-skips`: reschedule manually skipped items from State. By default, these skips are retained.
- `--resume-batch-mismatch POLICY`: the policy for differences in input root, output root, container, or encoding / decoding settings.
- `--resume-file-mismatch POLICY`: the policy for differences in source or completed output fingerprints.

Both policies default to `ask`. Available values are:

- `ask`: pause, display the differences, and wait for a choice. Enter `1 / 2 / 3 / 4` for Abort / Force / Skip / Restart. Append `all` to apply the choice to later differences of the same kind in this run.
- `abort`: stop recovery.
- `force`: ignore differences and trust reusable records.
- `skip`: skip affected items, record `resume_mismatch`, and retain the original records for a future choice.
- `restart`: restart the entire batch for settings-level differences, or reschedule the affected file for file-level differences. Both respect `--overwrite-output`.

Unattended execution, redirected input, and `--quiet` cannot answer `ask`; set both policies in advance. For full reuse criteria and the format, see the [State reference](../../docs/state.md).

## Predict: predictions for one configuration

```sh
yog -i input.mkv predict --encode-svt-av1 --preset 6 --quality 30
```

Encode samples using SVT-AV1 / preset 6 / CRF 30 to estimate the full video's size, encoding time, speed, VMAF, and other metrics.

Predict-specific options:

- `-r / --recursive`: predict directory inputs individually, with task error isolation and a summary.
- `--report PATH`: record predictions and sample data for each input.
- `--overwrite-report`: allow an existing Report to be replaced.

The `[prediction]` configuration controls sample count and duration. Normal output shows one summary line per item; `--verbose` expands the samples and prediction bounds.

## Emulate: quality curves and configuration comparisons

Emulate processes one input file and repeatedly runs the real prediction workflow. At least one of `--png PATH` and `--svg PATH` is required.

### One encoder and quality points

```sh
yog -i input.mkv emulate --png curve.png --svg curve.svg --range 18,23,28-30 --encode-x265 --preset medium
```

Predict at CRF 18, 23, 28, 29, and 30, generating PNG and SVG from the same data. `--range POINTS` accepts comma-separated discrete values and inclusive ascending ranges. It can be repeated; points are sorted and deduplicated. Omitting it or passing an empty string uses the encoder's full quality range.

Place encoder options after `--encode-*` to set the preset, device, or multipass mode. Emulate sets quality for each point.

### Multiple Candidates

Repeat `--candidate DECODE:ENCODE:PRESET:RANGE[:MULTIPASS]` to define multiple configurations:

- `DECODE`: `software / vaapi / cuda / qsv`. A hardware device can be specified as `vaapi=/dev/dri/renderD128`, for example.
- `ENCODE`: `libx264 / libx265 / libsvtav1 / libaom-av1 / librav1e`, or `h264/hevc/av1` followed by `_nvenc / _qsv / _vaapi`. A VAAPI encoding device can be specified as `hevc_vaapi=/dev/dri/renderD128`.
- `PRESET`: the corresponding preset; AOM uses a cpu-used number, and rav1e uses a speed number.
- `RANGE`: the same quality-point syntax as `--range`. Leave it empty for the full range, but retain the field and its colon.
- `MULTIPASS`: optional and supported only by NVENC; accepts `disabled / qres / fullres`.

```sh
yog -i input.mkv emulate --svg compare.svg --candidate software:libx265:medium:18,23,28 --candidate vaapi:libsvtav1:6:20-35
```

The first configuration uses software decoding / x265 / medium at three discrete CRF points. The second uses VAAPI decoding / SVT-AV1 / preset 6 at every CRF from 20 through 35.

### Charts, Report, and failures

`--overwrite-chart` permits replacing PNG / SVG files only; `--overwrite-report` permits replacing the Report only. `--report PATH` records one line per attempted quality point. Candidate indices start at 1.

The horizontal axis identifies the quality parameter, such as CRF / CQ / QP. The left axis shows VMAF and the right axis shows estimated size; each vertical axis uses its data's minimum and maximum as bounds. Solid lines represent VMAF, dashed lines represent size, and the same color identifies the same configuration. Multiple Candidates use numbered labels with configuration descriptions. Sizes use appropriate units, such as `800 MiB` and `1.1 GiB`. Dimensions are controlled by `[emulation].width / height`.

## Common option combinations

### Copy streams into a different container

```sh
yog -i input.mp4 -C mkv transcode -o output.mkv --copy
```

### Hardware decoding with software encoding

```sh
yog -i input.mkv --decode-vaapi=/dev/dri/renderD128 transcode -o output.mkv --encode-x265 --preset slow --quality 23
```

### NVENC target bitrate

```sh
yog -i input.mkv transcode -o output.mkv --encode-nvenc hevc --preset p5 --bitrate 4000000 --multipass fullres
```

### Batch transcoding with a Report

```sh
yog -i media transcode -r -o encoded -C mkv --report run-1.jsonl --encode-svt-av1 --preset 6 --quality 30
```

### Resume based on existing outputs, without State

```sh
yog -i media transcode -r -o encoded --no-state --copy
```

### Unattended recovery, leaving mismatched items for later

```sh
yog -i media transcode -r -o encoded --resume-batch-mismatch skip --resume-file-mismatch skip --report resumed.jsonl --encode-svt-av1 --preset 6 --quality 30
```

### Explicitly re-encode from scratch

```sh
yog -i media transcode -r -o encoded --restart --overwrite-output --encode-svt-av1 --preset 6 --quality 28
```

### Transcode, verify, score, and report

```sh
yog -i input.mkv transcode -o output.mkv --verify --vmaf=5 --report result.jsonl --encode-x264 --preset medium --quality 23
```

### Predict first, then run the full transcode

```sh
yog -i input.mkv predict --encode-svt-av1 --preset 6 --quality 30
yog -i input.mkv transcode -o output.mkv --encode-svt-av1 --preset 6 --quality 30
```

### Adjust sampling and inspect commands

```sh
yog -c config.toml -i input.mkv predict --verbose --encode-x265 --quality 23
```

## Exit codes and results

Success returns `0`, execution failure returns `1`, and Ctrl+C cancellation returns `130`. A batch returns its overall result after cleanup; the last file alone does not determine success. Actual task failures, State write errors, and recovery errors cause failure. A batch with no processable video also fails. Existing outputs, explicit manual skips, and policy-based skips do not trigger this "no video" failure.
