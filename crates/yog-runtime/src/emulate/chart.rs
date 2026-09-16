use super::{EmulationOptions, EmulationPoint};
use anyhow::Context;
use plotters::{
    coord::{
        Shift,
        ranged1d::{DefaultFormatting, KeyPointHint, Ranged},
    },
    prelude::*,
};
use std::fs;
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
    image_size: (u32, u32),
) -> anyhow::Result<()> {
    let VideoAction::Encode(encoding) = &request.video else {
        unreachable!("emulation arguments require a video encoder");
    };
    let title = title(encoding, &request.decoding);
    let quality_parameter = encoding.quality_parameter();
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
    let quality_values = [first.quality as f64, last.quality as f64];
    let quality_bounds = axis_bounds(quality_values[0], quality_values[1], 0.5);
    let (vmaf_minimum, vmaf_maximum) = value_bounds(points, |point| point.vmaf);
    let vmaf_bounds = axis_bounds(vmaf_minimum, vmaf_maximum, 0.5);
    let (size_minimum, size_maximum) =
        value_bounds(points, |point| point.size_bytes as f64 / MIB as f64);
    let size_padding = (size_minimum.abs() * 0.05).max(0.001);
    let mut size_bounds = axis_bounds(size_minimum, size_maximum, size_padding);
    size_bounds[0] = size_bounds[0].max(0.0);

    let quality_bold_points = quality_axis_points(quality_values, 11);
    let quality_light_points = quality_axis_points(quality_values, 64);
    let vmaf_bold_points = axis_points(vmaf_bounds, 7);
    let vmaf_light_points = axis_points(vmaf_bounds, 31);
    let size_bold_points = axis_points(size_bounds, 7);
    let size_light_points = axis_points(size_bounds, 31);

    root.fill(&WHITE)?;
    let mut chart = ChartBuilder::on(&root)
        .caption(title, ("sans-serif", 30))
        .margin(20)
        .set_label_area_size(LabelAreaPosition::Left, 70)
        .set_label_area_size(LabelAreaPosition::Right, 105)
        .set_label_area_size(LabelAreaPosition::Bottom, 55)
        .build_cartesian_2d(
            ExplicitAxis::new(
                quality_bounds,
                quality_bold_points.clone(),
                quality_light_points.clone(),
            ),
            ExplicitAxis::new(vmaf_bounds, vmaf_bold_points, vmaf_light_points),
        )?
        .set_secondary_coord(
            ExplicitAxis::new(quality_bounds, quality_bold_points, quality_light_points),
            ExplicitAxis::new(size_bounds, size_bold_points, size_light_points),
        );

    chart
        .configure_mesh()
        .x_desc(format!("{quality_parameter} value"))
        .y_desc("VMAF score")
        .x_labels(11)
        .y_labels(7)
        .light_line_style(RGBColor(225, 225, 225))
        .x_label_formatter(&|quality| format!("{quality:.0}"))
        .y_label_formatter(&|vmaf| format_vmaf(*vmaf, vmaf_bounds))
        .draw()?;
    chart
        .configure_secondary_axes()
        .x_labels(0)
        .y_labels(7)
        .y_desc("Estimated size")
        .y_label_formatter(&format_size)
        .draw()?;

    let vmaf_color = RGBColor(22, 163, 74);
    let size_color = RGBColor(202, 138, 4);
    chart
        .draw_series(LineSeries::new(
            points
                .iter()
                .map(|point| (point.quality as f64, point.vmaf)),
            &vmaf_color,
        ))?
        .label("VMAF score")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 28, y)], vmaf_color));

    chart
        .draw_secondary_series(LineSeries::new(
            points
                .iter()
                .map(|point| (point.quality as f64, point.size_bytes as f64 / MIB as f64)),
            &size_color,
        ))?
        .label("Estimated size")
        .legend(move |(x, y)| PathElement::new([(x, y), (x + 28, y)], size_color));

    chart
        .configure_series_labels()
        .position(SeriesLabelPosition::UpperRight)
        .background_style(WHITE.mix(0.85))
        .border_style(BLACK)
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

fn axis_bounds(minimum: f64, maximum: f64, padding: f64) -> [f64; 2] {
    if minimum < maximum {
        [minimum, maximum]
    } else {
        [minimum - padding, maximum + padding]
    }
}

fn axis_points(bounds: [f64; 2], count: usize) -> Vec<f64> {
    let last = count - 1;
    (0..count)
        .map(|index| bounds[0] + (bounds[1] - bounds[0]) * index as f64 / last as f64)
        .collect()
}

fn quality_axis_points(bounds: [f64; 2], maximum_points: usize) -> Vec<f64> {
    let start = bounds[0].round() as u8;
    let end = bounds[1].round() as u8;
    if start >= end {
        return vec![f64::from(start)];
    }

    let step = usize::from(end - start).div_ceil(maximum_points - 1);
    let mut points = (usize::from(start)..=usize::from(end))
        .step_by(step)
        .map(|value| value as f64)
        .collect::<Vec<_>>();
    if points.last().copied() != Some(f64::from(end)) {
        points.push(f64::from(end));
    }
    points
}

fn format_vmaf(value: f64, bounds: [f64; 2]) -> String {
    if bounds[1] - bounds[0] < 0.1 {
        format!("{value:.3}")
    } else {
        format!("{value:.2}")
    }
}

fn format_size(size_mib: &f64) -> String {
    if *size_mib >= 1024.0 {
        format!("{:.1} GiB", size_mib / 1024.0)
    } else {
        format!("{size_mib:.1} MiB")
    }
}

#[derive(Clone)]
struct ExplicitAxis {
    bounds: [f64; 2],
    bold_points: Vec<f64>,
    light_points: Vec<f64>,
}

impl ExplicitAxis {
    fn new(bounds: [f64; 2], bold_points: Vec<f64>, light_points: Vec<f64>) -> Self {
        Self {
            bounds,
            bold_points,
            light_points,
        }
    }
}

impl Ranged for ExplicitAxis {
    type FormatOption = DefaultFormatting;
    type ValueType = f64;

    fn map(&self, value: &f64, limit: (i32, i32)) -> i32 {
        let position = (*value - self.bounds[0]) / (self.bounds[1] - self.bounds[0]);
        (f64::from(limit.0) + f64::from(limit.1 - limit.0) * position).round() as i32
    }

    fn key_points<Hint: KeyPointHint>(&self, hint: Hint) -> Vec<f64> {
        if hint.weight().allow_light_points() {
            self.light_points.clone()
        } else {
            self.bold_points.clone()
        }
    }

    fn range(&self) -> std::ops::Range<f64> {
        self.bounds[0]..self.bounds[1]
    }
}
