use anyhow::Context;
use clap::{ArgGroup, Parser, ValueEnum};
use std::{num::NonZeroU32, num::NonZeroUsize, path::PathBuf};
use yog_runtime::dev_tools::emulation_chart::{Point, Series, check_paths, render};

const MIB: f64 = 1024.0 * 1024.0;
const SOURCE_MIB: f64 = 800.0;

#[derive(Debug, Parser)]
#[command(about = "Render an emulation chart from deterministic mock data")]
#[command(group(
    ArgGroup::new("output")
        .args(["png", "svg"])
        .required(true)
        .multiple(true)
))]
struct Args {
    #[arg(long, group = "output", value_name = "PATH")]
    png: Option<PathBuf>,
    #[arg(long, group = "output", value_name = "PATH")]
    svg: Option<PathBuf>,
    #[arg(long, default_value = "2")]
    candidates: NonZeroUsize,
    #[arg(long, default_value = "18,40", value_parser = parse_range)]
    range: QualityRange,
    #[arg(short, long, value_enum, default_value_t = QualityParameter::Crf)]
    parameter: QualityParameter,
    #[arg(long, default_value = "1280")]
    width: NonZeroU32,
    #[arg(long, default_value = "720")]
    height: NonZeroU32,
}

#[derive(Debug, Clone, Copy)]
struct QualityRange {
    start: u8,
    end: u8,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum QualityParameter {
    Crf,
    Cq,
    Qp,
}

impl QualityParameter {
    fn label(self) -> &'static str {
        match self {
            Self::Crf => "CRF",
            Self::Cq => "CQ",
            Self::Qp => "QP",
        }
    }

    fn candidate_label(self, index: usize) -> String {
        let labels = match self {
            Self::Crf => [
                "SVT-AV1 / Software / Preset 6",
                "x265 / Software / Preset slow",
                "libaom-AV1 / Software / CPU Used 4",
                "rav1e / Software / Speed 6",
            ],
            Self::Cq => [
                "AV1 NVENC / NVDEC / Preset p5 / Multipass quarter-resolution",
                "HEVC NVENC / NVDEC / Preset p6 / Multipass full-resolution",
                "AV1 QSV / QSV / Preset medium",
                "HEVC VAAPI / VAAPI / Device /dev/dri/renderD128",
            ],
            Self::Qp => [
                "H.264 / Software / Constant QP",
                "HEVC / Software / Constant QP",
                "H.264 QSV / QSV / Preset medium",
                "HEVC VAAPI / VAAPI / Device /dev/dri/renderD128",
            ],
        };
        let label = labels[(index - 1) % labels.len()];
        if index <= labels.len() {
            label.to_owned()
        } else {
            format!("{label} / Variant {}", (index - 1) / labels.len() + 1)
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let parameter = args.parameter.label();
    let series = mock_series(args.candidates.get(), args.range, args.parameter, parameter);

    check_paths(args.png.as_deref(), args.svg.as_deref(), true)
        .context("cannot prepare preview output")?;
    render(
        args.png.as_deref(),
        args.svg.as_deref(),
        &series,
        (SOURCE_MIB * MIB) as u64,
        (args.width.get(), args.height.get()),
    )
    .context("cannot render chart preview")?;

    if let Some(path) = args.png {
        println!("wrote {}", path.display());
    }
    if let Some(path) = args.svg {
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn parse_range(value: &str) -> Result<QualityRange, String> {
    let Some((start, end)) = value.split_once(',') else {
        return Err("expected START,END".to_owned());
    };
    if end.contains(',') {
        return Err("expected START,END".to_owned());
    }

    let start = start
        .trim()
        .parse::<u8>()
        .map_err(|_| "range start must be an integer from 0 to 255".to_owned())?;
    let end = end
        .trim()
        .parse::<u8>()
        .map_err(|_| "range end must be an integer from 0 to 255".to_owned())?;
    if start >= end {
        return Err("range end must be greater than its start".to_owned());
    }
    Ok(QualityRange { start, end })
}

fn mock_series(
    count: usize,
    range: QualityRange,
    quality_parameter: QualityParameter,
    parameter: &'static str,
) -> Vec<Series> {
    (1..=count)
        .map(|index| {
            let curve = (index - 1) % 8;
            let vmaf_offset = curve as f64 * 0.45;
            let size_factor = 0.92 + curve as f64 * 0.025;
            let span = f64::from(range.end - range.start);
            let points = (range.start..=range.end)
                .map(|quality| {
                    let position = f64::from(quality - range.start) / span;
                    let vmaf = (99.0 - 24.0 * position.powf(1.55) + vmaf_offset).clamp(0.0, 100.0);
                    let size_mib = SOURCE_MIB * (1.45 - 1.15 * position.powf(0.8)) * size_factor;
                    Point {
                        quality,
                        vmaf,
                        size_bytes: (size_mib * MIB).round() as u64,
                    }
                })
                .collect();

            Series {
                index: Some(index),
                label: quality_parameter.candidate_label(index),
                parameter,
                points,
            }
        })
        .collect()
}
