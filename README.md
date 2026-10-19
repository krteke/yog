# yog

A video transcoding tool built on FFmpeg / FFprobe, with a CLI, TUI, and desktop GUI. Supports batch processing, sample-based predictions, quality curves, and VMAF scoring.

## Documentation

- [CLI guide](crates/yog-cli/README.md): common options, Transcode / Predict / Emulate, and common option combinations.
- [Advanced reference](docs/README.md): format specifications for scripting and further development.
- [Report format](docs/report.md): JSONL results, measurement fields, and result classifications.
- [State format and recovery](docs/state.md): batch state, reuse criteria, and recovery choices.

## Getting started

```sh
cargo run -p yog-tui
```

Launch the TUI. The CLI feature is enabled by default; when arguments are provided, execution is delegated to the CLI. For example: `cargo run -p yog-tui -- --help`.

```sh
cargo run -p yog-cli -- --help
```

Use the standalone CLI directly.

```sh
cargo run -p yog-gpui
```

Launch the desktop GUI. Before running media tasks, install FFmpeg / FFprobe and make sure your local build provides the required encoders, hardware backends, and `libvmaf`.
