use anyhow::Context;
use plotters::{
    coord::{
        Shift,
        ranged1d::{DefaultFormatting, KeyPointHint, Ranged},
    },
    prelude::*,
};
use std::{fs, path::Path};

const MIB: f64 = 1024.0 * 1024.0;

pub struct Point {
    pub quality: u8,
    pub vmaf: f64,
    pub size_bytes: u64,
}

pub struct Series {
    pub index: Option<usize>,
    pub label: String,
    pub parameter: &'static str,
    pub points: Vec<Point>,
}

const COLORS: [RGBColor; 8] = [
    RGBColor(22, 163, 74),
    RGBColor(37, 99, 235),
    RGBColor(202, 138, 4),
    RGBColor(220, 38, 38),
    RGBColor(147, 51, 234),
    RGBColor(13, 148, 136),
    RGBColor(219, 39, 119),
    RGBColor(101, 163, 13),
];

fn colors(count: usize) -> impl Iterator<Item = RGBColor> {
    COLORS.into_iter().cycle().take(count)
}

pub fn check_paths(png: Option<&Path>, svg: Option<&Path>, overwrite: bool) -> anyhow::Result<()> {
    for path in [png, svg].into_iter().flatten() {
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

pub fn render(
    png: Option<&Path>,
    svg: Option<&Path>,
    series: &[Series],
    source_bytes: u64,
    image_size: (u32, u32),
) -> anyhow::Result<()> {
    let title = title(series);
    let parameter = parameter(series);
    if let Some(path) = png {
        draw(
            BitMapBackend::new(path, image_size).into_drawing_area(),
            &title,
            &parameter,
            series,
            source_bytes,
        )
        .with_context(|| format!("cannot render PNG {}", path.display()))?;
    }
    if let Some(path) = svg {
        draw(
            SVGBackend::new(path, image_size).into_drawing_area(),
            &title,
            &parameter,
            series,
            source_bytes,
        )
        .with_context(|| format!("cannot render SVG {}", path.display()))?;
    }
    Ok(())
}

fn parameter(series: &[Series]) -> String {
    let mut parameters = Vec::new();
    for series in series {
        if !parameters.contains(&series.parameter) {
            parameters.push(series.parameter);
        }
    }
    parameters.join(" / ")
}

fn title(series: &[Series]) -> String {
    let indices = series
        .iter()
        .filter_map(|series| series.index)
        .collect::<Vec<_>>();
    if !indices.is_empty() && indices.len() == series.len() {
        let indices = indices
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(" / ");
        return if series.len() == 1 {
            format!("Candidate {indices}")
        } else {
            format!("Candidates {indices}")
        };
    }

    match series {
        [single] => single.label.clone(),
        _ => series
            .iter()
            .map(|series| series.label.as_str())
            .collect::<Vec<_>>()
            .join(" vs "),
    }
}

fn draw<DB>(
    root: DrawingArea<DB, Shift>,
    title: &str,
    quality_parameter: &str,
    series: &[Series],
    source_bytes: u64,
) -> anyhow::Result<()>
where
    DB: DrawingBackend,
    DB::ErrorType: 'static,
{
    let (Some(minimum), Some(maximum)) = (
        points(series).map(|point| point.quality).min(),
        points(series).map(|point| point.quality).max(),
    ) else {
        anyhow::bail!("cannot render an emulation chart without quality points");
    };

    let quality_values = [f64::from(minimum), f64::from(maximum)];
    let quality_bounds = axis_bounds(quality_values[0], quality_values[1], 0.5);
    let (vmaf_minimum, vmaf_maximum) = value_bounds(series, |point| point.vmaf);
    let vmaf_bounds = axis_bounds(vmaf_minimum, vmaf_maximum, 0.5);
    let (size_minimum, size_maximum) = value_bounds(series, |point| point.size_bytes as f64 / MIB);
    let source_size = source_bytes as f64 / MIB;
    let show_source_size = (size_minimum..=size_maximum).contains(&source_size);
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
    let chart_root = root.titled(title, ("sans-serif", 30))?;
    let description_rows = series
        .iter()
        .filter(|series| series.index.is_some())
        .count();
    let description_height = 30 + description_rows as u32 * 22;
    let (descriptions, chart_root) = chart_root.split_vertically(description_height);
    draw_descriptions(&descriptions, series, show_source_size, source_size)?;

    let mut chart = ChartBuilder::on(&chart_root)
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

    for (item, color) in series.iter().zip(colors(series.len())) {
        if item.points.is_empty() {
            continue;
        }
        chart.draw_series(LineSeries::new(
            item.points
                .iter()
                .map(|point| (point.quality as f64, point.vmaf)),
            color,
        ))?;

        chart.draw_secondary_series(DashedLineSeries::new(
            item.points
                .iter()
                .map(|point| (point.quality as f64, point.size_bytes as f64 / MIB)),
            6,
            4,
            color.stroke_width(1),
        ))?;
    }

    if show_source_size {
        let source_color = RGBColor(96, 96, 96).mix(0.7);
        chart.draw_secondary_series(DashedLineSeries::new(
            [
                (quality_bounds[0], source_size),
                (quality_bounds[1], source_size),
            ],
            6,
            4,
            source_color.stroke_width(1),
        ))?;
    }

    root.present()?;
    Ok(())
}

fn draw_descriptions<DB>(
    area: &DrawingArea<DB, Shift>,
    series: &[Series],
    show_source_size: bool,
    source_size: f64,
) -> anyhow::Result<()>
where
    DB: DrawingBackend,
    DB::ErrorType: 'static,
{
    let mut row = 0;
    for (item, color) in series.iter().zip(colors(series.len())) {
        let Some(index) = item.index else {
            continue;
        };
        let y = 10 + row * 22;
        area.draw(&Rectangle::new([(20, y - 6), (34, y + 6)], color.filled()))?;
        area.draw(&Text::new(
            format!("{index}: {}", item.label),
            (44, y),
            ("sans-serif", 16),
        ))?;
        row += 1;
    }

    let guide = if show_source_size {
        format!(
            "Solid: VMAF    Dashed: estimated size    Gray dashed: input size ({})",
            format_size(&source_size)
        )
    } else {
        "Solid: VMAF    Dashed: estimated size".to_owned()
    };
    area.draw(&Text::new(guide, (20, 10 + row * 22), ("sans-serif", 15)))?;
    Ok(())
}

fn points(series: &[Series]) -> impl Iterator<Item = &Point> {
    series.iter().flat_map(|item| item.points.iter())
}

fn value_bounds(series: &[Series], value: impl Fn(&Point) -> f64) -> (f64, f64) {
    let mut values = points(series).map(value);
    let first = values
        .next()
        .expect("emulation charts are rendered with at least one point");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_labels_switch_from_mib_to_gib_at_one_gib() {
        assert_eq!(format_size(&800.0), "800.0 MiB");
        assert_eq!(format_size(&1024.0), "1.0 GiB");
        assert_eq!(format_size(&(1.1 * 1024.0)), "1.1 GiB");
    }

    #[test]
    fn candidate_title_uses_only_candidate_numbers() {
        let series = [
            Series {
                index: Some(1),
                label: "SVT-AV1 / Software / Preset 4".to_owned(),
                parameter: "CRF",
                points: Vec::new(),
            },
            Series {
                index: Some(2),
                label: "x265 / Software / Preset slow".to_owned(),
                parameter: "CRF",
                points: Vec::new(),
            },
        ];

        assert_eq!(title(&series), "Candidates 1 / 2");
    }

    #[test]
    fn candidate_chart_has_color_descriptions_without_point_markers() {
        let series = [
            Series {
                index: Some(1),
                label: "SVT-AV1 / Software / Preset 4".to_owned(),
                parameter: "CRF",
                points: vec![
                    Point {
                        quality: 20,
                        vmaf: 95.0,
                        size_bytes: 800 * 1024 * 1024,
                    },
                    Point {
                        quality: 21,
                        vmaf: 94.0,
                        size_bytes: 760 * 1024 * 1024,
                    },
                ],
            },
            Series {
                index: Some(2),
                label: "x265 / Software / Preset slow".to_owned(),
                parameter: "CRF",
                points: vec![
                    Point {
                        quality: 20,
                        vmaf: 96.0,
                        size_bytes: 820 * 1024 * 1024,
                    },
                    Point {
                        quality: 21,
                        vmaf: 95.0,
                        size_bytes: 780 * 1024 * 1024,
                    },
                ],
            },
        ];
        let mut svg = String::new();
        draw(
            SVGBackend::with_string(&mut svg, (800, 600)).into_drawing_area(),
            &title(&series),
            "CRF",
            &series,
            900 * 1024 * 1024,
        )
        .unwrap();

        assert!(svg.contains("Candidates 1 / 2"));
        assert!(svg.contains("1: SVT-AV1 / Software / Preset 4"));
        assert!(svg.contains("2: x265 / Software / Preset slow"));
        assert!(svg.contains("Solid: VMAF    Dashed: estimated size"));
        assert!(!svg.contains("<circle"));
    }
}
