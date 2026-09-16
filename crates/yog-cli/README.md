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
