use super::support::Fixture;
use std::fs;

fn command_quality_count(commands: &str, encoder: &str, quality: u8) -> usize {
    commands
        .lines()
        .filter(|command| {
            command.contains(encoder) && command.contains(&format!("-crf:v {quality}"))
        })
        .count()
}

fn emulation_fixture(ffmpeg: &str) -> Fixture {
    let fixture = Fixture::new(ffmpeg);
    fs::write(fixture.0.join("input.mkv"), b"12345678901234567890").unwrap();
    fixture.tool(
        "ffprobe",
        r#"packets=false
for argument do
    test "$argument" = -show_packets && packets=true
done
for last do :; done
if test "$packets" = true; then
    printf '%s' '{"packets":[{"stream_index":0,"size":"10"}]}'
elif test "$last" = input.mkv; then
    printf '%s' '{"streams":[{"index":0,"codec_type":"video","width":320,"height":180,"pix_fmt":"yuv420p"}],"format":{"format_name":"matroska,webm","duration":"1"},"pixel_formats":[{"name":"yuv420p","nb_components":3,"log2_chroma_w":1,"log2_chroma_h":1,"flags":{"rgb":0,"alpha":0,"palette":0,"hwaccel":0},"components":[{"bit_depth":8},{"bit_depth":8},{"bit_depth":8}]}]}'
else
    printf '%s' '{"streams":[{"index":0,"codec_type":"video"}],"format":{"duration":"1"}}'
fi"#,
    );
    fixture
}

fn measured_emulation_fixture() -> Fixture {
    emulation_fixture(
        r#"printf '%s\n' "$*" >> ffmpeg-commands
filter=''
for argument do
    case "$argument" in
        encoder=*)
            printf '%s\n' "Encoder ${argument#encoder=} [test]" '    Supported pixel formats: yuv420p'
            exit 0
            ;;
        *libvmaf=*) filter=$argument ;;
    esac
done
if test -n "$filter"; then
    metrics=${filter#*log_path=\'}
    metrics=${metrics%%\':shortest=*}
    printf '%s' '{"frames":[{"frameNum":0,"metrics":{"vmaf":96.5,"float_ssim":0.99,"psnr_y":42.0}}]}' > "$metrics"
    exit 0
fi
for last do :; done
printf 12345678901234567890 > "$last"
printf 'out_time_us=1000000\nspeed=1x\nprogress=end\n'"#,
    )
}

#[path = "emulation/candidates.rs"]
mod candidates;
#[path = "emulation/charts.rs"]
mod charts;
#[path = "emulation/reports.rs"]
mod reports;
