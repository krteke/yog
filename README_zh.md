# yog

基于 FFmpeg / FFprobe 的视频转码工具，提供 CLI、TUI 和桌面 GUI；支持批处理、抽样预估、质量曲线和 VMAF 评分。

## 文档

- [CLI 使用指南](crates/yog-cli/README.md)：通用参数、Transcode / Predict / Emulate 和常用组合。
- [高级参考](docs/README.md)：面向脚本集成和二次开发的格式说明。
- [Report 输出格式](docs/report.md)：JSONL 结果、测量字段与状态分类。
- [State 格式与恢复](docs/state.md)：批处理状态、复用条件与恢复选择。

## 启动

```sh
cargo run -p yog-tui
```

启动 TUI。默认启用 CLI feature，有参数时委托给 CLI；例如 `cargo run -p yog-tui -- --help`。

```sh
cargo run -p yog-cli -- --help
```

直接使用独立 CLI。

```sh
cargo run -p yog-gpui
```

启动桌面 GUI。运行媒体任务前，请安装 FFmpeg / FFprobe，并确认本机构建提供所需编码器、硬件后端和 `libvmaf`。
