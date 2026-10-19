# Report JSONL 参考

## 文件与写入时机

通过 `--report PATH` 开启。文件为 UTF-8 JSONL：每行一个完整 JSON 对象，以换行结束。

默认只创建新文件，已有文件需要 `--overwrite-report` 才能清空重写。一次运行中逐条写入并刷新，取消或失败后已写入的行保留。创建失败会阻止运行，运行中写入或最终刷新失败产生诊断警告，不撤销已发布视频。

最外层只有一个类型键：`transcode`、`predict`、`emulate` 或 `skipped`。例如：

```json
{"skipped":{"input":"/media/input/movie.mkv","result":{"status":"skipped","reason":"existing_output","message":"output file already exists: /media/output/movie.mkv"}}}
```

批处理中的探测/资格跳过、State 复用和未启动的手动跳过使用 `skipped`，已经进入转码流程的任务也可能在 `transcode.result` 中记录手动跳过。

Transcode / Predict 通常每个处理项一行，Emulate 每个已尝试的质量点一行。

## 共同约定

字节字段使用原始 bytes，时长为秒，码率为 bit/s，速度为“媒体秒 / 实际耗时秒”。

未知或尚未测得的可选字段通常为 `null`，数组为空时为 `[]`。`skipped.recovery`、`vmaf.skip_reason` 在未使用时直接省略。失败记录也可能保留已经取得的源信息。

### `result`：结果分类

成功：`{"status":"success"}`。

跳过：`{"status":"skipped","reason":"no_video","message":"no regular video streams"}`。

失败：`{"status":"failed","phase":"transcoding","timed_out":false,"message":"transcode failed: ..."}`。

取消：`{"status":"cancelled"}`。

`reason` 的含义：

- `manual`：用户手动跳过，或保留之前的手动跳过。
- `existing_output`：批处理发现最终输出文件已存在且未授权覆盖。
- `already_completed`：State 中的成功记录被复用，正常匹配或用户明确 Force 后信任记录。
- `resume_mismatch`：恢复发现差异，用户或预设策略选择 Skip。
- `no_video`：没有常规视频流。
- `probe_failed`：FFprobe 无法探测该批处理输入。
- `probe_timed_out`：FFprobe 探测超时。
- `unsupported_container`：无法选择受支持的输出容器，通常需要显式指定容器。

`phase` 使用以下蛇形字符串：`discovering`、`probing`、`preparing_output`、`planning`、`predicting`、`transcoding`、`verifying`、`publishing`、`calculating_vmaf`、`rendering_chart`。

`timed_out` 是结构化超时分类。

### `video`：请求的编码设置

- `action`：`copy` 或 `encode`。
- `encoder`：FFmpeg 编码器名，例如 `libx265`、`hevc_nvenc`，复制时为 `null`。
- `rate`：`null`，或 `{ "kind": "quality", "parameter": "CRF", "value": 23 }` 一类对象。`kind` 为 `quality / bitrate`，质量参数为 `CRF / CQ / QP / global_quality`，码率参数为 `b`。`value` 是质量等级或 bit/s。
- `preset`：字符串或 `null`。命名预设如 `medium`、`p5`，SVT-AV1 / AOM / rav1e 的数字也写成字符串，如 `"6"`，分别代表 preset / cpu-used / speed。
- `multipass`：NVENC 的 `disabled / qres / fullres`，否则 `null`。
- `device`：VAAPI 编码设备显示路径，否则 `null`。

速率或预设为 `null` 表示未显式指定。复制时其余编码字段均为 `null`。

`decoding` 是显示字符串：`software`、`vaapi`、`cuda`、`qsv`，指定设备后形如 `vaapi:/dev/dri/renderD128`。

`container` 是解析后的容器扩展名，例如 `mkv`，解析前失败时可为 `null`。

### `source`：源媒体事实

`source` 为对象或 `null`，包含：

- `bytes`：源文件大小，无法取得时 `null`。
- `duration_seconds`：合法正数媒体时长，未知时 `null`。
- `streams`：`video / audio / subtitle / attachment / cover / other` 的数量。`cover` 是 attached picture，不重复计入 `video`。
- `video`：第一个常规视频流，或 `null`。字段为 `codec`、`width`、`height`、`frame_rate`、`bit_rate`。后两者保留 FFprobe 的字符串，例如 `"24000/1001"`、`"1500000"`。
- `audio`：全部音频流的数组，每项有 `codec`、`channels`、`sample_rate`，其中采样率保留字符串，例如 `"48000"`。

## `transcode` 记录

包含 `input`、`output`、`result`、`container`、`video`、`decoding`、`source`，以及以下三组可选信息。

### `metrics`

成功发布后取得的测量：

- `bytes`：最终文件大小。
- `duration_seconds`：FFmpeg 最后进度中报告的输出时长，缺少时为 `null`。
- `size_percent`：输出 bytes / 源 bytes × 100，可以大于 100，缺少有效大小时为 `null`。
- `elapsed_seconds`：实际 FFmpeg 转码运行耗时，不包含整批扫描、前置探测、后续验证和 VMAF 的全部时间。
- `speed`：处理时长 / FFmpeg 转码耗时，优先使用进度时长，缺少时可用源媒体时长。
- `frames`、`fps`：最后一次进度中的帧数、fps，缺少时为 `null`。

没有发布成功的结果通常没有 `metrics`。

### `verify`

未请求验证时为 `null`，请求后含 `outcome` 与 `warnings`：

- `complete`：验证完成。
- `missing_audio_after_seek`：验证定位到缺失音频，属于确定的验证失败。
- `incomplete`：未能完整验证，原因记在 `warnings`，不必然使转码失败。

最终是否成功以 `result` 为准。

### `vmaf`

未请求评分时为 `null`，请求后含 `n_subsample`、`score`、`error`，可能还有 `skip_reason`。`n_subsample: null` 表示逐帧，N 表示每 N 帧采样。

成功时 `score` 有值，失败或取消时 `error` 记录说明。用户只跳过评分阶段时 `skip_reason` 为 `manual`。

## `predict` 记录

包含共同的 `input / result / container / video / decoding / source`，没有视频 `output`，另有：

- `sampling.requested_samples`、`sample_seconds`：请求样本数和每段期望时长。
- `sampling.measured_samples`、`sampled_seconds`：实际测得的样本数及总时长。
- `speed`：速度估计。
- `transcode_seconds`：整片转码耗时估计，秒。
- `output_bytes`：整片输出体积估计，bytes。
- `size_percent`：中心体积估计 / 源大小 × 100。
- `quality`：抽样质量汇总，包含 `frames`（评分帧数）、`source_stream_index`（评分使用的源流索引）、`vmaf`、`ssim`、`psnr_y_db`。
- `samples`：原始样本测量数组。

各估计使用 `{ "value": 96.5, "low": 95.0, "high": 97.2 }`，分别为中心估计和上下界，不是正式整片测量，也不是固定置信水平的保证。SSIM / PSNR-Y 未计算或不可用时为 `null`。

每个样本字段为 `start_seconds`、`duration_seconds`、`encode_seconds`、`speed`、`timed_payload_bytes`、`scored_frames`、`vmaf`、`ssim`、`psnr_y_db`。

失败预估仍保留请求信息，估计对象可能全部为 `null`、`samples` 为 `[]`、实际样本数为零。

## `emulate` 记录

在同一个对象中平铺 Predict 的所有字段，另加：

- `candidate_index`：Candidate 的一基编号，使用单编码器子命令时为 `null`。
- `outputs`：`png` 和 `svg` 的请求绝对路径，未请求的一种为 `null`。

`video.rate` 写实际尝试的质量点，因此能用 `candidate_index + video.rate.value` 分组。每组按质量点升序写入，取消时只保留已经尝试的点。失败点也有自己的 `result`，成功点仍可生成图表。

## `skipped` 与恢复信息

最小记录只有 `input` 和 `result`。State 复用或恢复策略跳过时还含：

```json
{"recovery":{"forced":false,"output":"/media/output/movie.mkv"}}
```

这是记录内部的一段，不是独立的 Report 行。`forced` 表示用户忽略恢复差异而信任记录，`output` 是恢复记录提供的输出位置，也可能为 `null`。不是每种跳过都有这个对象。

## 示例：流式统计

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
