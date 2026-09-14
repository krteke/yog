# yog CLI

```sh
cargo run -p yog-cli -- input.mkv -o output.mkv --encode-x264 --preset medium --quality 23
cargo run -p yog-cli -- input.mkv -o output.mkv --copy
cargo run -p yog-cli -- input.mkv -o output.mkv --decode-vaapi=/dev/dri/renderD128 --encode-nvenc h264
cargo run -p yog-cli -- input.mkv -o output.mkv --encode-x265 --preset slow --bitrate 4000000
cargo run -p yog-cli -- input.mkv --encode-vaapi hevc -o output.mkv --device /dev/dri/renderD128 --quality 27
cargo run -p yog-cli -- --help
cargo run -p yog-cli -- --encode-nvenc --help
cargo run -p yog-cli -- input.mkv -o output.mkv --encode-x264 --verify
cargo run -p yog-cli -- input.mkv -o output.mkv --encode-x264 --vmaf
cargo run -p yog-cli -- input.mkv -o output.mkv --encode-x264 --vmaf=5
cargo run -p yog-cli -- input.mkv -o output.mkv --predict --encode-x264 --quality 23
cargo run -p yog-cli -- input.mp4 -o output.mp4 -C mp4 -v --copy
```

[config.toml](./config.example.toml)
