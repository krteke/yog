# State JSONL 与批处理恢复

State 记录“哪些项可以沿用、哪些需要重新执行”。它只用于递归 Transcode，Predict、Emulate 和单文件 Transcode 不使用。操作参数见 [CLI 指南](../crates/yog-cli/README.md)，测量结果格式见 [Report 参考](report.md)。

## 入口和职责

默认路径为输出根目录下的 `.yog-state.jsonl`，`--state PATH` 指定别处，`--no-state` 同时关闭读写。文件不存在时自动创建。

关闭 State 后，批处理仍会在未授权覆盖时跳过已有最终文件，理由为 `existing_output`。State 复用成功记录的理由则是 `already_completed`，需要正常匹配或用户明确选择 Force。

## 文件布局

UTF-8 JSONL，第一行是 Header，其余行是 Entry，**没有 `header` / `entry` 类型包装键**。同一个 `relative` 可出现多次，最后一条是当前有效记录。

运行前完成恢复选择后，会直接重写同一 State：保留文件头和各路径的最新记录，再在运行中追加变化。例如同一文件曾依次有 `running → running(part=true) → success` 三行，下次开始会只留下最新 `success`，随后新任务继续追加。

每次写行后刷新缓冲，但不执行每行 `fsync`。不完整的最后一行可被忽略并警告，其他 JSON 解析错误会中止读取。`--restart` 可以放弃不可读取的 State，重新建立记录。

### 路径表示

State 路径使用操作系统原生字符串的 Serde 表示，以保存非 UTF-8 文件名：

```json
{"Unix":[47,109,101,100,105,97]}
```

在 Unix 上这是 `/media` 的原始字节。Windows 使用 `{"Windows":[...]}`，内容为原生宽字符单元。

输入根目录通过文件系统解析为绝对路径，输出根目录和指定的 State 路径使用项目的绝对路径解析。Entry 的 `relative` 相对于输入根目录，`output.path` 是最终输出绝对路径。

### Header

包含 `input_root` 和 `settings`：

- `input_root`：原生路径表示的输入根目录。
- `settings`：去重的参数组数组，Entry 通过零基 `setting` 引用。

每组 Settings 含：

- `output_root`：原生路径表示的输出根目录。
- `container`：`mkv`、`mp4` 等容器值，`null` 表示自动选择。
- `video`：核心 `VideoAction` 的序列化形式。
- `decoding`：核心 `DecodingBackend` 的序列化形式。

例如视频设置可以是：

```json
{"action":"encode","encoder":"x265","rate":{"kind":"quality","value":23},"preset":"medium"}
```

复制是 `{"action":"copy"}`。编码器标记为 `x264 / x265 / svt_av1 / aom_av1 / rav1e / nvenc / qsv / vaapi`，不是 Report 的 `libx265` 一类名字。速率为 `null` 或带 `kind: quality / bitrate` 的对象，值在 `value` 中。

x264 / x265 / QSV / NVENC 的 `preset` 是名称，SVT-AV1 的 `preset`、AOM 的 `cpu_used`、rav1e 的 `speed` 是整数或 `null`。硬件编码器另有 `codec: h264 / hevc / av1`，NVENC 还记录 `multipass: disabled / qres / fullres` 或 `null`，VAAPI 记录原生路径 `device`。

软件解码为 `{"backend":"software"}`，硬件解码例如 `{"backend":"vaapi","device":null}`，指定设备时 `device` 是原生操作系统字符串对象。

### Entry

每条任务记录含：

- `relative`：原生路径，相对输入根目录，也是“最后记录覆盖前记录”的键。
- `setting`：本条任务使用的 Settings 数组下标，从 0 开始。
- `input`：源文件指纹，取不到时 `null`。
- `output`：含 `path`（原生最终输出路径）与 `fingerprint`（指纹或 `null`）。
- `result`：保存的任务状态，见下文。
- `part`：是否有该任务认领的半成品需要在重试时清理。

指纹形如：

```json
{"bytes":1048576,"modified":{"secs_since_epoch":1760000000,"nanos_since_epoch":123456789}}
```

`bytes` 是长度，`modified` 是文件系统修改时间，`nanos_since_epoch` 是秒以内的纳秒部分。

### 保存的 `result`

- `{"status":"running"}`：已开始，尚无终态。
- `{"status":"success"}`：视频已发布。
- `{"status":"skipped","reason":"manual"}`：跳过，完整原因清单见 [Report 参考](report.md)。
- `{"status":"failed","phase":"transcoding","timed_out":false}`：失败及阶段。
- `{"status":"cancelled"}`：视频发布前取消。

### 一项任务如何写入

正常流程先追加 `running, part=false`，认领 `.part.movie.mkv` 后追加 `running, part=true`，发布 `movie.mkv` 后追加 `success, part=false` 并记录输出指纹。失败、取消、手动跳过也追加各自的终态。

只有 State 认领过的半成品才会进入恢复清理范围，`--overwrite-output` 本身不表示可以删除任意已有 `.part.*`。State 写入失败会记录状态错误，停止继续派发后续任务。

## 下次启动时的判定

路径扫描后，按相对路径查找最新 Entry。默认可复用 `success`，以及原因是 `no_video / unsupported_container / manual` 的跳过。`running / failed / cancelled` 等记录重新调度，`probe_failed / probe_timed_out / existing_output` 也不会被当作可信的已完成视频复用。

成功复用输出 `Skipped · Already completed`，批次计入 reused，保留的手动跳过仍计入 skipped / manual。没有可复用记录的已有文件仍由输出覆盖策略决定：未授权则 Existing output 跳过。

重新调度不等于授权覆盖。默认已有最终视频会跳过，想重新编码，需另开 `--overwrite-output`。反过来，仅开启覆盖也不会使匹配的 State 成功记录自动失效。

### 参数与指纹差异

一般比较输入根目录、输出根目录、容器、视频操作及编码参数，仅在编码时比较解码器。对于 `no_video / manual` 跳过，不要求编码输出设置一致，对于 `unsupported_container`，设置比较只关心容器。

文件级比较源文件指纹，成功记录还比较保存路径上的输出指纹。缺少指纹、文件无法访问或大小/时间不同会作为差异交给策略决定，。

两个策略分别由 `--resume-batch-mismatch` 和 `--resume-file-mismatch` 控制：

- 默认 `ask`：等待用户回答，等待期间可以取消。CLI 需要交互终端，TUI / GUI 使用恢复确认界面。
- `abort`：结束恢复。
- `force`：忽略差异并信任可复用记录，原本失败或运行中的项仍需重做。
- `skip`：本次绕过受影响项，结果为 `resume_mismatch`。
- `restart`：设置级重新调度整批，文件级只重新调度受影响项。若最终输出还在，仍遵守输出覆盖设置。

CLI 可在选择后加 `all`，UI 可选择应用到同类差异，只在本次运行有效。无人值守时应预设非 ask 策略。

### 手动跳过与重新开始

TUI / GUI 可跳过当前项，或当前项起指定数量的待执行项，未执行项不启动子进程，运行中项取消当前阶段。Report / State 记录 `manual`。若视频已发布、正在计算 VMAF，则只跳过评分，视频保持成功。

默认沿用已有手动跳过，`--retry-manual-skips` 重新调度它们。失败项、已由 State 完成的项不应因跳过配额被重新标为手动跳过。

`--restart` 显式放弃之前的复用判定，可以用于输入目录搬迁后按当前目录重新开始，不是无条件强制源路径必须相同。它不会赋予输出覆盖权限，通常真正从头重编码需要 `--restart --overwrite-output`。
