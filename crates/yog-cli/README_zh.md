# Yog CLI

Yog 通过 FFmpeg / FFprobe 处理视频，提供三个子命令：`transcode` 实际转码，`predict` 对一组参数抽样预估，`emulate` 对多个质量等级或多组参数预估并绘制曲线。

## 运行方式

```sh
cargo run -p yog-cli -- --help
```

查看某个操作或编码器的帮助：

```sh
yog transcode --help
yog transcode --encode-nvenc --help
```

第一条显示转码参数，第二条显示 NVENC 编码参数。编码器也可以用普通子命令名选择，例如 `x264`，本文统一使用 `--encode-x264` 这种写法。

运行前需要能找到 FFmpeg、FFprobe。所选编码器、硬件后端、滤镜必须受本机 FFmpeg 和驱动支持，Predict、Emulate 和转码后的 VMAF 计算需要 `libvmaf`。

## 通用参数

这些参数适用于三个子命令，可以放在操作名称前后，也可以放在编码器子命令后。

### 输入：`-i / --input PATH`

每次运行必须指定输入。普通模式传一个文件，`transcode --recursive` 和 `predict --recursive` 传目录。

### 容器：`-C / --container FORMAT`

显式选择封装格式。支持 `mkv`、`mp4`、`mov`、`m4a`、`3gp`、`3g2`、`f4v`、`ismv`、`psp`、`webm`、`ts`、`m2ts`、`avi`、`flv`、`asf`、`wmv`、`mpg`（别名 `mpeg`）、`vob`、`ogg`、`ogv`。

不指定时根据输入文件及 FFprobe 格式信息选择。单文件模式请让输出后缀与容器一致，批处理指定容器后，会统一替换输出后缀。

Predict / Emulate 用它选择抽样目标容器，容器开销也是预估条件的一部分。

### 解码器

- 不传硬件解码参数：软件解码。
- `--decode-vaapi[=DEVICE]`：VAAPI 解码，例如 `--decode-vaapi=/dev/dri/renderD128`。
- `--decode-cuda[=DEVICE]`：CUDA 解码，例如 `--decode-cuda=0`。
- `--decode-qsv[=DEVICE]`：QSV 解码，可以指定设备。

三者互斥。设备可省略，如果指定，必须用 `=` 连接。解码器和编码器独立选择，VAAPI 解码可以配合 x265 软件编码。

### 配置文件：`-c / --config PATH`

读取 TOML 配置。未指定时尝试系统配置目录下的 `yog/config.toml`，Linux 通常是 `$XDG_CONFIG_HOME/yog/config.toml`，未设置时通常是 `~/.config/yog/config.toml`。自动寻找的文件不存在时使用默认值，显式指定的文件不存在则报错。

[配置示例](config.example.toml)

`progress_tick_interval_ms` 是终端进度刷新间隔，单位毫秒。`prediction.samples` 是期望样本数，`sample_sec` 是每段期望时长，单位秒，Predict 和 Emulate 共用，实际样本受媒体长度等条件影响。`emulation.width / height` 是曲线图宽高，单位像素。省略的项使用默认值，未知配置项会报错。

### 工具、超时与输出

- `--ffmpeg PATH`、`--ffprobe PATH`：程序路径，默认分别在 PATH 中寻找 `ffmpeg`、`ffprobe`。
- `-t / --timeout SECONDS`：每次子进程执行的时限，默认不限时。
- `--verbose`：详细诊断和完整预估信息，包括区间，默认日志级别提高到 Debug，显示真实执行命令及 FFmpeg 日志。普通模式显示进度、Predict spinner 和简短摘要。
- `--quiet`：关闭运行期终端输出和日志，与 `--verbose` 互斥。仍执行任务、写指定的 Report / State、返回退出码，Clap 参数解析错误仍会显示。
- `-h / --help`：当前层级的帮助，顶层 `-V / --version` 显示版本。

日志可通过 `RUST_LOG` 筛选，例如 `RUST_LOG=yog_core=debug yog ...`，`--quiet` 时不会重新开启日志

## Transcode：实际转码

```sh
yog -i input.mkv transcode -o output.mkv --encode-x265 --quality 23 --preset medium
```

用 x265 编码视频，质量为 CRF 23，预设为 medium。不选择视频操作时默认 `--copy`，即复制视频流。音频、字幕和附件等由转码计划按容器能力映射、复制。

### 输出与批处理

`-o / --output PATH` 必填：单文件模式是最终文件路径，`-r / --recursive` 模式是输出根目录。批处理保留输入的相对目录结构，不指定容器时也保留文件名，指定容器后替换后缀。

批处理先扫描路径并检查输出冲突，再逐文件探测、执行。无视频、探测失败、无法选择容器的输入记为跳过。视频转码失败会记录并继续，最后汇总，路径映射冲突、无法准备 Report 等整批准备错误仍会阻止启动，手动取消停止后续任务并汇总。

未启用覆盖时，批处理遇到已有最终输出文件，记为 **Skipped · Existing output**，继续下一项。单文件模式仍对已有输出报错。

`--overwrite-output` 允许替换最终视频。先写同目录的 `.part.文件名`，例如 `.part.movie.mkv`，成功后才发布为 `movie.mkv`。未被恢复流程认领的已有 `.part.*` 不会直接被覆盖。

### 编码器与速率控制

编码器选项也用于 Predict，Emulate 使用编码器、预设等设置，质量由质量点决定。

`-q / --quality N` 选择编码器对应的质量控制，`-b / --bitrate N` 指定正整数视频码率，单位 **bit/s**，例如 `4000000`，两者互斥。不传时按 FFmpeg / 编码器默认速率设置执行。

- `--copy`：仅 Transcode 可用，复制视频。
- `--encode-x264`、`--encode-x265`：`libx264`、`libx265`，CRF 范围 `0–51`。`-p / --preset` 可选 `ultrafast / superfast / veryfast / faster / fast / medium / slow / slower / veryslow`。
- `--encode-svt-av1`：`libsvtav1`，CRF 范围 `0–63`，`-p / --preset` 范围 `0–13`。
- `--encode-aom-av1`：`libaom-av1`，CRF 范围 `0–63`，`-u / --cpu-used` 范围 `0–8`，控制速度与编码效率。
- `--encode-rav1e`：`librav1e`，QP 范围 `0–255`，`-s / --speed` 范围 `0–10`。
- `--encode-nvenc CODEC`：必填 `h264 / hevc / av1`。CQ 在 H.264、HEVC 下为 `0–51`，AV1 下为 `0–63`，`-p / --preset` 为 `p1–p7`，`-m / --multipass` 为 `disabled / qres / fullres`，分别是关闭、四分之一分辨率分析、全分辨率分析。
- `--encode-qsv CODEC`：必填 `h264 / hevc / av1`。`global_quality` 范围 `1–51`，`-p / --preset` 为 `veryfast / faster / fast / medium / slow / slower / veryslow`。
- `--encode-vaapi CODEC`：必填 `h264 / hevc / av1`。H.264、HEVC 使用 QP，范围 `0–52`，AV1 使用 `global_quality`，范围 `0–255`。`-d / --device PATH` 选择编码设备，默认 `/dev/dri/renderD128`，没有 preset。

### 验证与 VMAF

`-v / --verify` 在发布前检查转码结果，包括流和必要的媒体验证，复制模式还利用包信息核对。确定的验证失败会导致该项失败并保留已有目标，无法完整验证时可能只产生警告，结论写入 Report 的 `verify`。

`--vmaf` 或 `--vmaf=full` 逐帧计算 VMAF，`--vmaf=N` 每 N 帧采样一次，N 为正整数，例如 `--vmaf=5`。可选值必须用 `=` 指定。

VMAF 在视频成功发布后独立执行。VMAF 线程数按可用 CPU 并行度设置，目前没有独立 CLI 线程选项。

### Report

`--report PATH` 写 JSONL 结果，包含状态、编码参数、源媒体事实和已获得的测量。`--overwrite-report` 允许清空重写已有 Report，未授权时已有文件阻止本次运行。

### State 与继续批处理

以下选项仅用于 `transcode --recursive`：

- 默认读取输出目录的 `.yog-state.jsonl`，不存在则创建。匹配的完成记录显示 **Already completed**，不重新转码，不同于只看文件存在的 **Existing output**。
- `--state PATH`：指定状态文件。
- `--no-state`：既不读也不写 State，与其余恢复选项互斥。仍执行“已有输出跳过”。
- `--restart`：放弃之前的完成/跳过判定，按当前参数重新调度，不自动授权覆盖视频。真正重编码已有输出要同时传 `--overwrite-output`。
- `--retry-manual-skips`：重新调度 State 中手动跳过的项，默认保留这些跳过。
- `--resume-batch-mismatch POLICY`：输入根目录、输出根目录、封装或编解码设置不一致时的策略。
- `--resume-file-mismatch POLICY`：源文件或已完成输出的指纹不一致时的策略。

两个策略默认均为 `ask`，可选：

- `ask`：暂停、显示差异并等待选择。输入 `1 / 2 / 3 / 4` 选择 Abort / Force / Skip / Restart，加 `all` 可应用于本次运行后续同类差异。
- `abort`：终止恢复。
- `force`：忽略差异，信任可复用的记录。
- `skip`：跳过受影响项，记录 `resume_mismatch`，保留原记录供以后选择。
- `restart`：设置级差异重新开始整批，文件级差异重新调度该文件。均遵守 `--overwrite-output`。

无人值守、重定向输入或 `--quiet` 时不能回答 `ask`，请预先指定两个策略。完整复用条件和格式见 [State 参考](../../docs/state.md)。

## Predict：单组参数预估

```sh
yog -i input.mkv predict --encode-svt-av1 --preset 6 --quality 30
```

使用 SVT-AV1 / preset 6 / CRF 30 抽样编码，估计整片体积、耗时、速度及 VMAF 等指标。

Predict 自己的参数：

- `-r / --recursive`：逐个预估目录输入，保留错误隔离、汇总。
- `--report PATH`：记录各输入的预估及样本数据。
- `--overwrite-report`：允许替换已有 Report。

样本数和时长由 `[prediction]` 配置。普通输出每项一行摘要，`--verbose` 展开样本和估计上下限。

## Emulate：质量曲线与参数对比

只处理一个输入文件，反复调用真实预估流程。必须指定 `--png PATH`、`--svg PATH` 至少一个。

### 单个编码器与质量点

```sh
yog -i input.mkv emulate --png curve.png --svg curve.svg --range 18,23,28-30 --encode-x265 --preset medium
```

在 CRF 18、23、28、29、30 分别预估，同一数据生成 PNG 和 SVG。`--range POINTS` 支持逗号分隔离散值和两端包含的升序范围，可重复传入，排序去重。不传或传空字符串时使用编码器完整质量范围。

编码器参数放在 `--encode-*` 后，可设置预设、设备、multipass，质量由 Emulate 逐点设置。

### 多组 Candidate

可重复传 `--candidate DECODE:ENCODE:PRESET:RANGE[:MULTIPASS]`，每次定义一组：

- `DECODE`：`software / vaapi / cuda / qsv`，硬件设备可写 `vaapi=/dev/dri/renderD128` 等。
- `ENCODE`：`libx264 / libx265 / libsvtav1 / libaom-av1 / librav1e`，或 `h264/hevc/av1` 加 `_nvenc / _qsv / _vaapi`。VAAPI 编码设备可写 `hevc_vaapi=/dev/dri/renderD128`。
- `PRESET`：对应预设，AOM 用 cpu-used 数字，rav1e 用 speed 数字。
- `RANGE`：与 `--range` 相同的质量点语法，留空表示完整范围，但字段及冒号仍需保留。
- `MULTIPASS`：可省略，仅 NVENC 支持，值为 `disabled / qres / fullres`。

```sh
yog -i input.mkv emulate --svg compare.svg --candidate software:libx265:medium:18,23,28 --candidate vaapi:libsvtav1:6:20-35
```

第一组软件解码 / x265 / medium，在三个离散 CRF 点预估，第二组 VAAPI 解码 / SVT-AV1 / preset 6，逐点计算 CRF 20–35。

### 图表、Report 与失败

`--overwrite-chart` 只允许覆盖 PNG / SVG，`--overwrite-report` 只允许覆盖 Report。`--report PATH` 每个已尝试的质量点记录一行，Candidate 编号从 1 开始。

横轴注明 CRF / CQ / QP 等参数，左轴为 VMAF，右轴为预估体积，各纵轴按数据最小、最大值定范围。实线为 VMAF，虚线为体积，同颜色表示同组参数，多 Candidate 用编号并附参数说明。体积按量级显示为 `800 MiB`、`1.1 GiB` 等。尺寸由 `[emulation].width / height` 控制。

## 常用参数组合

### 复制流并换封装

```sh
yog -i input.mp4 -C mkv transcode -o output.mkv --copy
```

### 硬件解码、软件编码

```sh
yog -i input.mkv --decode-vaapi=/dev/dri/renderD128 transcode -o output.mkv --encode-x265 --preset slow --quality 23
```

### NVENC 目标码率

```sh
yog -i input.mkv transcode -o output.mkv --encode-nvenc hevc --preset p5 --bitrate 4000000 --multipass fullres
```

### 批量转码并记录结果

```sh
yog -i media transcode -r -o encoded -C mkv --report run-1.jsonl --encode-svt-av1 --preset 6 --quality 30
```

### 不使用 State，只按已有输出继续

```sh
yog -i media transcode -r -o encoded --no-state --copy
```

### 无人值守恢复，差异项暂不处理

```sh
yog -i media transcode -r -o encoded --resume-batch-mismatch skip --resume-file-mismatch skip --report resumed.jsonl --encode-svt-av1 --preset 6 --quality 30
```

### 明确从头重编码

```sh
yog -i media transcode -r -o encoded --restart --overwrite-output --encode-svt-av1 --preset 6 --quality 28
```

### 转码、验证、评分和报告

```sh
yog -i input.mkv transcode -o output.mkv --verify --vmaf=5 --report result.jsonl --encode-x264 --preset medium --quality 23
```

### 先预估，再正式运行

```sh
yog -i input.mkv predict --encode-svt-av1 --preset 6 --quality 30
yog -i input.mkv transcode -o output.mkv --encode-svt-av1 --preset 6 --quality 30
```

### 调整抽样并查看命令

```sh
yog -c config.toml -i input.mkv predict --verbose --encode-x265 --quality 23
```

## 退出和结果

成功返回 `0`，执行失败返回 `1`，Ctrl+C 取消返回 `130`。批处理在收尾后按整批结果返回，不能只看最后一个文件。有实际失败、State 写入错误或恢复错误时返回失败，完全没有可处理视频的批次也返回失败。已有输出、显式手动跳过或策略跳过不会触发这种“没有视频”的失败。
