# State JSONL and batch recovery

State records which items can be reused and which need to run again. It is used only for recursive Transcode, not for Predict, Emulate, or single-file Transcode. For operation options, see the [CLI guide](../crates/yog-cli/README.md); for measurement results, see the [Report reference](report.md).

## Entry points and responsibilities

The default path is `.yog-state.jsonl` under the output root. Use `--state PATH` to choose another location, or `--no-state` to disable both reading and writing. The file is created automatically when missing.

With State disabled, batch processing still skips existing final files without overwrite permission, using the reason `existing_output`. Reusing a successful State record uses `already_completed`, requiring either a normal match or an explicit Force choice.

## File layout

UTF-8 JSONL: the first line is a Header and subsequent lines are Entries, with **no `header` / `entry` type wrapper keys**. The same `relative` path may occur multiple times; its last entry is the currently effective record.

After recovery choices are resolved and before execution starts, the same State file is rewritten directly, retaining the header and the latest record for each path. Changes are then appended during execution. For example, if a file previously had three lines representing `running → running(part=true) → success`, only the latest `success` remains at the next start, and new tasks continue appending afterward.

The buffer is flushed after each line, without a per-line `fsync`. An incomplete final line may be ignored with a warning; other JSON parsing errors stop reading. `--restart` can discard unreadable State and establish new records.

### Path representation

State paths use Serde's representation of native operating-system strings to preserve non-UTF-8 filenames:

```json
{"Unix":[47,109,101,100,105,97]}
```

On Unix, these are the raw bytes of `/media`. Windows uses `{"Windows":[...]}`, containing native wide-character units.

The input root is resolved through the filesystem to an absolute path. The output root and specified State path use the project's absolute-path resolution. An Entry's `relative` is relative to the input root, and `output.path` is the absolute final output path.

### Header

Contains `input_root` and `settings`:

- `input_root`: the input root in native path representation.
- `settings`: a deduplicated array of configurations, referenced by an Entry's zero-based `setting`.

Each Settings object contains:

- `output_root`: the output root in native path representation.
- `container`: a container value such as `mkv` or `mp4`; `null` means automatic selection.
- `video`: the serialized core `VideoAction`.
- `decoding`: the serialized core `DecodingBackend`.

For example, video settings can be:

```json
{"action":"encode","encoder":"x265","rate":{"kind":"quality","value":23},"preset":"medium"}
```

Copying is `{"action":"copy"}`. Encoder tags are `x264 / x265 / svt_av1 / aom_av1 / rav1e / nvenc / qsv / vaapi`, not Report names such as `libx265`. Rate is `null` or an object with `kind: quality / bitrate` and its value in `value`.

The x264 / x265 / QSV / NVENC `preset` is a name. SVT-AV1's `preset`, AOM's `cpu_used`, and rav1e's `speed` are integers or `null`. Hardware encoders also include `codec: h264 / hevc / av1`. NVENC additionally records `multipass: disabled / qres / fullres` or `null`; VAAPI records `device` as a native path.

Software decoding is `{"backend":"software"}`. Hardware decoding looks like `{"backend":"vaapi","device":null}`; when specified, `device` is a native operating-system string object.

### Entry

Each task record contains:

- `relative`: a native path relative to the input root, also the key by which later records replace earlier ones.
- `setting`: the zero-based Settings array index used by this task.
- `input`: the source file fingerprint, or `null` if unavailable.
- `output`: contains `path` (the native final output path) and `fingerprint` (a fingerprint or `null`).
- `result`: the saved task status, described below.
- `part`: whether a partial output claimed by this task needs cleanup on retry.

A fingerprint looks like:

```json
{"bytes":1048576,"modified":{"secs_since_epoch":1760000000,"nanos_since_epoch":123456789}}
```

`bytes` is the file length, `modified` is the filesystem modification time, and `nanos_since_epoch` is the subsecond nanosecond component.

### Saved `result`

- `{"status":"running"}`: started, with no terminal status yet.
- `{"status":"success"}`: the video has been published.
- `{"status":"skipped","reason":"manual"}`: skipped; see the [Report reference](report.md) for the full list of reasons.
- `{"status":"failed","phase":"transcoding","timed_out":false}`: failure and its phase.
- `{"status":"cancelled"}`: cancelled before video publication.

### How a task is recorded

The normal workflow first appends `running, part=false`, then `running, part=true` after claiming `.part.movie.mkv`. Publishing `movie.mkv` appends `success, part=false` and records the output fingerprint. Failures, cancellations, and manual skips also append their respective terminal statuses.

Only partial outputs claimed by State are eligible for recovery cleanup. `--overwrite-output` alone does not authorize deleting arbitrary existing `.part.*` files. A State write failure records a state error and stops dispatching subsequent tasks.

## Decisions at the next start

After path scanning, recovery looks up the latest Entry by relative path. By default, `success` records and skips with reasons `no_video / unsupported_container / manual` can be reused. Records such as `running / failed / cancelled` are rescheduled. `probe_failed / probe_timed_out / existing_output` are not reused as trusted completed videos either.

Reusing a successful record emits `Skipped · Already completed` and counts toward the batch's reused total. Retained manual skips still count toward skipped / manual totals. Existing files without reusable records remain subject to the output overwrite policy: without permission, they are skipped as Existing output.

Rescheduling does not grant overwrite permission. Existing final videos are skipped by default; re-encoding requires a separate `--overwrite-output`. Conversely, enabling overwrite alone does not invalidate matching successful State records.

### Settings and fingerprint differences

Recovery generally compares input root, output root, container, video action, and encoding settings. Decoders are compared only when encoding. For `no_video / manual` skips, encoding and output settings need not match. For `unsupported_container`, settings comparison considers only the container.

File-level checks compare source fingerprints. Successful records also compare the output fingerprint at the saved path. Missing fingerprints, inaccessible files, or different sizes / times are treated as differences to be resolved by policy.

The two policies are controlled by `--resume-batch-mismatch` and `--resume-file-mismatch`:

- Default `ask`: wait for a user response; cancellation remains available while waiting. The CLI requires an interactive terminal, while TUI / GUI use recovery confirmation interfaces.
- `abort`: stop recovery.
- `force`: ignore differences and trust reusable records. Previously failed or running items still need to run again.
- `skip`: bypass affected items for this run, with the result `resume_mismatch`.
- `restart`: reschedule the entire batch for settings-level differences, or only the affected item for file-level differences. Existing final outputs still follow the output overwrite setting.

In the CLI, append `all` to a choice; in the UI, select the option to apply it to similar differences. This applies only to the current run. For unattended execution, preset a policy other than ask.

### Manual skips and restarting

The TUI / GUI can skip the current item or a specified number of pending items starting with the current one. Unstarted items do not launch subprocesses; running items cancel their current phase. Report / State record `manual`. If the video has already been published and VMAF is running, only scoring is skipped; the video remains successful.

Previous manual skips are retained by default; `--retry-manual-skips` reschedules them. Failed items and items already completed according to State should not be relabeled as manually skipped because of a skip quota.

`--restart` explicitly discards previous reuse decisions. It can be used to start again from the current directory after moving an input directory, rather than unconditionally requiring the source path to remain the same. It does not grant output overwrite permission; actually re-encoding from scratch usually requires `--restart --overwrite-output`.
