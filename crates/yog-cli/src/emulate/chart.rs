use super::{EmulationOptions, EmulationPoint};
use crate::config;
use anyhow::Context;
use plotters::{coord::Shift, prelude::*};
use std::{fs, ops::Range};
use yog_core::ffmpeg::{
    decoding::DecodingBackend,
    encoding::{VideoCodec, VideoEncoding},
    plan::{TranscodeRequest, VideoAction},
};

const MIB: u64 = 1_048_576;

pub(super) fn check_paths(options: &EmulationOptions, overwrite: bool) -> anyhow::Result<()> {
    for path in [&options.png, &options.svg].into_iter().flatten() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create directory {}", parent.display()))?;
        }
        anyhow::ensure!(
            overwrite || !path.exists(),
            "{} already exists",
            path.display()
        );
    }
    Ok(())
}

pub(super) fn render(
    request: &TranscodeRequest,
    options: &EmulationOptions,
    points: &[EmulationPoint],
) -> anyhow::Result<()> {
    let VideoAction::Encode(encoding) = &request.video else {
        unreachable!("emulation arguments require a video encoder");
    };
    let title = title(encoding, &request.decoding);
    let quality_parameter = encoding.quality_parameter();
    let emulation = &config::get().emulation;
    let image_size = (emulation.width.get(), emulation.height.get());

    if let Some(path) = &options.png {
        draw(
            BitMapBackend::new(path, image_size).into_drawing_area(),
            &title,
            quality_parameter,
            points,
        )
        .with_context(|| format!("cannot render PNG {}", path.display()))?;
    }
    if let Some(path) = &options.svg {
        draw(
            SVGBackend::new(path, image_size).into_drawing_area(),
            &title,
            quality_parameter,
            points,
        )
        .with_context(|| format!("cannot render SVG {}", path.display()))?;
    }
    Ok(())
}

fn title(encoding: &VideoEncoding, decoding: &DecodingBackend) -> String {
    let mut parts = vec![
        match encoding {
            VideoEncoding::X264 { .. } => "x264".to_owned(),
            VideoEncoding::X265 { .. } => "x265".to_owned(),
            VideoEncoding::SvtAv1 { .. } => "SVT-AV1".to_owned(),
            VideoEncoding::AomAv1 { .. } => "AOM-AV1".to_owned(),
            VideoEncoding::Rav1e { .. } => "rav1e".to_owned(),
            VideoEncoding::Nvenc { codec, .. } => {
                format!("{} NVENC", codec_name(*codec))
            }
            VideoEncoding::Qsv { codec, .. } => format!("{} QSV", codec_name(*codec)),
            VideoEncoding::Vaapi { codec, .. } => format!("{} VAAPI", codec_name(*codec)),
        },
        match decoding {
            DecodingBackend::Software => "Software",
            DecodingBackend::Vaapi(_) => "VAAPI decode",
            DecodingBackend::Cuda(_) => "CUDA decode",
            DecodingBackend::Qsv(_) => "QSV decode",
        }
        .to_owned(),
    ];

    match encoding {
        VideoEncoding::X264 {
            preset: Some(preset),
            ..
        }
        | VideoEncoding::X265 {
            preset: Some(preset),
            ..
        } => parts.push(format!("Preset {}", preset.as_str())),
        VideoEncoding::SvtAv1 {
            preset: Some(preset),
            ..
        } => parts.push(format!("Preset {preset}")),
        VideoEncoding::AomAv1 {
            cpu_used: Some(cpu_used),
            ..
        } => parts.push(format!("CPU Used {cpu_used}")),
        VideoEncoding::Rav1e {
            speed: Some(speed), ..
        } => parts.push(format!("Speed {speed}")),
        VideoEncoding::Nvenc {
            preset, multipass, ..
        } => {
            if let Some(preset) = preset {
                parts.push(format!("Preset {}", preset.as_str()));
            }
            if let Some(multipass) = multipass {
                parts.push(format!("Multipass {}", multipass.as_str()));
            }
        }
        VideoEncoding::Qsv {
            preset: Some(preset),
            ..
        } => parts.push(format!("Preset {}", preset.as_str())),
        VideoEncoding::Vaapi { .. }
        | VideoEncoding::X264 { preset: None, .. }
        | VideoEncoding::X265 { preset: None, .. }
        | VideoEncoding::SvtAv1 { preset: None, .. }
        | VideoEncoding::AomAv1 { cpu_used: None, .. }
        | VideoEncoding::Rav1e { speed: None, .. }
        | VideoEncoding::Qsv { preset: None, .. } => {}
    }

    parts.join(" / ")
}

fn codec_name(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "H.264",
        VideoCodec::Hevc => "HEVC",
        VideoCodec::Av1 => "AV1",
    }
}

fn draw<DB>(
    root: DrawingArea<DB, Shift>,
    title: &str,
    quality_parameter: &str,
    points: &[EmulationPoint],
) -> anyhow::Result<()>
where
    DB: DrawingBackend,
    DB::ErrorType: 'static,
{
    let first = points
        .first()
        .expect("emulation quality range always produces at least one point");
    let last = points
        .last()
        .expect("emulation quality range always produces at least one point");
    let quality_axis = axis_range(first.quality as f64, last.quality as f64, 0.5);
    let (vmaf_minimum, vmaf_maximum) = value_bounds(points, |point| point.vmaf);
    let vmaf_axis = axis_range(vmaf_minimum, vmaf_maximum, 0.5);
    let (size_minimum, size_maximum) =
        value_bounds(points, |point| point.size_bytes as f64 / MIB as f64);
    let size_padding = (size_minimum.abs() * 0.05).max(0.001);
    let mut size_axis = axis_range(size_minimum, size_maximum, size_padding);
    size_axis.start = size_axis.start.max(0.0);

    root.fill(&WHITE)?;
    let mut chart = ChartBuilder::on(&root)
        .caption(title, ("sans-serif", 30))
        .margin(24)
        .set_label_area_size(LabelAreaPosition::Left, 72)
        .set_label_area_size(LabelAreaPosition::Right, 84)
        .set_label_area_size(LabelAreaPosition::Bottom, 56)
        .build_cartesian_2d(quality_axis.clone(), vmaf_axis)?
        .set_secondary_coord(quality_axis, size_axis);

    chart
        .configure_mesh()
        .x_desc(format!("{quality_parameter} value"))
        .y_desc("VMAF score")
        .x_labels(points.len().min(16))
        .x_label_formatter(&|quality| {
            let rounded = quality.round();
            if (quality - rounded).abs() < 1e-6 {
                format!("{rounded:.0}")
            } else {
                String::new()
            }
        })
        .axis_desc_style(("sans-serif", 20))
        .label_style(("sans-serif", 16))
        .draw()?;
    chart
        .configure_secondary_axes()
        .x_labels(0)
        .y_desc("Estimated size")
        .y_label_formatter(&format_size)
        .axis_desc_style(("sans-serif", 20))
        .label_style(("sans-serif", 16))
        .draw()?;

    chart
        .draw_series(LineSeries::new(
            points
                .iter()
                .map(|point| (point.quality as f64, point.vmaf)),
            BLUE.stroke_width(3),
        ))?
        .label("VMAF score")
        .legend(|(x, y)| PathElement::new([(x, y), (x + 28, y)], BLUE.stroke_width(3)));
    chart.draw_series(
        points
            .iter()
            .map(|point| Circle::new((point.quality as f64, point.vmaf), 3, BLUE.filled())),
    )?;

    chart
        .draw_secondary_series(LineSeries::new(
            points
                .iter()
                .map(|point| (point.quality as f64, point.size_bytes as f64 / MIB as f64)),
            RED.stroke_width(3),
        ))?
        .label("Estimated size")
        .legend(|(x, y)| PathElement::new([(x, y), (x + 28, y)], RED.stroke_width(3)));
    chart.draw_secondary_series(points.iter().map(|point| {
        Circle::new(
            (point.quality as f64, point.size_bytes as f64 / MIB as f64),
            3,
            RED.filled(),
        )
    }))?;

    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperRight)
        .background_style(WHITE.mix(0.85))
        .border_style(BLACK)
        .label_font(("sans-serif", 16))
        .draw()?;
    root.present()?;
    Ok(())
}

fn value_bounds(points: &[EmulationPoint], value: impl Fn(&EmulationPoint) -> f64) -> (f64, f64) {
    let mut values = points.iter().map(value);
    let first = values
        .next()
        .expect("emulation quality range always produces at least one point");
    values.fold((first, first), |(minimum, maximum), value| {
        (minimum.min(value), maximum.max(value))
    })
}

fn axis_range(minimum: f64, maximum: f64, padding: f64) -> Range<f64> {
    if minimum < maximum {
        minimum..maximum
    } else {
        minimum - padding..maximum + padding
    }
}

fn format_size(size_mib: &f64) -> String {
    if *size_mib >= 1024.0 {
        format!("{:.1} GiB", size_mib / 1024.0)
    } else {
        format!("{size_mib:.1} MiB")
    }
}
