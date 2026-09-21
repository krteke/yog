mod chart;

use crate::{
    TaskOutcome,
    diagnostics::Diagnostics,
    error::RunError,
    progress::Display,
    record_task,
    report::{
        Report,
        record::{EmulateRecord, OutputsRecord},
    },
    transcode::Transcoder,
};
use anyhow::Context;
use std::path::PathBuf;
use yog_core::ffmpeg::{
    decoding::DecodingBackend,
    encoding::VideoEncoding,
    plan::{TranscodeRequest, VideoAction},
};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub decoding: DecodingBackend,
    pub encoding: VideoEncoding,
    pub qualities: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct EmulationOptions {
    pub png: Option<PathBuf>,
    pub svg: Option<PathBuf>,
    pub qualities: Vec<u8>,
    pub candidates: Vec<Candidate>,
}

pub struct EmulationJob {
    candidate: Candidate,
    index: Option<usize>,
}

impl EmulationJob {
    pub fn label(&self) -> String {
        label(&self.candidate.decoding, &self.candidate.encoding)
    }

    pub fn is_candidate(&self) -> bool {
        self.index.is_some()
    }

    pub fn parameter(&self) -> &'static str {
        self.candidate.encoding.quality_parameter()
    }

    pub fn qualities(&self) -> &[u8] {
        &self.candidate.qualities
    }
}

fn label(decoding: &DecodingBackend, encoding: &VideoEncoding) -> String {
    let mut parts = vec![encoding.to_string(), decoding.name()];

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
        VideoEncoding::Vaapi { device, .. } => {
            parts.push(format!("Device {}", device.display()));
        }
        VideoEncoding::X264 { preset: None, .. }
        | VideoEncoding::X265 { preset: None, .. }
        | VideoEncoding::SvtAv1 { preset: None, .. }
        | VideoEncoding::AomAv1 { cpu_used: None, .. }
        | VideoEncoding::Rav1e { speed: None, .. }
        | VideoEncoding::Qsv { preset: None, .. } => {}
    }

    parts.join(" / ")
}

fn jobs(request: &TranscodeRequest, options: &EmulationOptions) -> Vec<EmulationJob> {
    if options.candidates.is_empty() {
        let VideoAction::Encode(encoding) = &request.video else {
            unreachable!("emulation arguments require a video encoder");
        };

        return vec![EmulationJob {
            candidate: Candidate {
                decoding: request.decoding.clone(),
                encoding: encoding.clone(),
                qualities: select(&options.qualities, encoding),
            },
            index: None,
        }];
    }

    options
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let mut candidate = candidate.clone();
            candidate.qualities = select(&candidate.qualities, &candidate.encoding);
            EmulationJob {
                candidate,
                index: Some(index + 1),
            }
        })
        .collect()
}

fn select(qualities: &[u8], encoding: &VideoEncoding) -> Vec<u8> {
    let mut qualities = if qualities.is_empty() {
        encoding.quality_range().collect()
    } else {
        qualities.to_vec()
    };
    qualities.sort_unstable();
    qualities.dedup();
    qualities
}

struct EmulationPoint {
    quality: u8,
    vmaf: f64,
    size_bytes: u64,
}

impl Transcoder {
    pub async fn emulate(
        &self,
        request: TranscodeRequest,
        options: &EmulationOptions,
        diagnostics: &Diagnostics,
        report: &mut Report,
    ) -> Result<(), RunError> {
        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        if !request.input.exists() {
            return Err(RunError::Failed(anyhow::anyhow!(
                "input file does not exist"
            )));
        }
        let source_bytes = request
            .input
            .metadata()
            .with_context(|| format!("cannot read input metadata {}", request.input.display()))?
            .len();

        chart::check_paths(options, request.overwrite)
            .context("cannot prepare emulation chart output")?;
        let media = self.probe(&request.input).await?;
        let prediction_options = self.config.prediction.into();
        let jobs = jobs(&request, options);
        let total = jobs.iter().map(|job| job.qualities().len()).sum::<usize>();
        let outputs = OutputsRecord::new(options.png.as_deref(), options.svg.as_deref());
        let progress = Display::emulating(self.progress_visible(), self.tick_interval());
        let mut series = Vec::with_capacity(jobs.len());
        let mut visited = 0;
        let mut succeeded = 0;
        let mut failures = Vec::new();

        for job in &jobs {
            let mut sample_request = request.clone();
            sample_request.decoding = job.candidate.decoding.clone();
            sample_request.video = VideoAction::Encode(job.candidate.encoding.clone());
            let label = job.label();
            let parameter = job.parameter();
            let mut points = Vec::with_capacity(job.qualities().len());

            for &quality in job.qualities() {
                if self.cancelled() {
                    return Err(RunError::Cancelled);
                }
                visited += 1;
                progress.emulate_quality(&label, parameter, quality, visited, total);

                let VideoAction::Encode(encoding) = &mut sample_request.video else {
                    unreachable!("emulation arguments require a video encoder");
                };
                encoding.set_quality(quality);

                let mut record = EmulateRecord::new(
                    &sample_request,
                    quality,
                    job.index,
                    outputs.clone(),
                    prediction_options,
                );
                record.fill_source(&sample_request, &media);
                let result = self
                    .predict_result(&sample_request, &media, prediction_options, diagnostics)
                    .await;
                let outcome = match result {
                    Ok(prediction) => {
                        record.fill_prediction(&prediction);
                        points.push(EmulationPoint {
                            quality,
                            vmaf: prediction.quality.vmaf.value,
                            size_bytes: prediction.output_bytes.value,
                        });
                        succeeded += 1;
                        TaskOutcome::Success
                    }
                    Err(error) => error
                        .context(format!("prediction failed at {parameter} {quality}"))
                        .into(),
                };
                record_task(record, &outcome, report, diagnostics);

                match outcome {
                    TaskOutcome::Success => {}
                    TaskOutcome::Failed(error) => failures.push(error),
                    TaskOutcome::Cancelled => return Err(RunError::Cancelled),
                }
            }

            series.push(chart::Series {
                index: job.index,
                label,
                parameter,
                points,
            });
        }
        drop(progress);

        if self.cancelled() {
            return Err(RunError::Cancelled);
        }
        if succeeded == 0 {
            let error = failures
                .into_iter()
                .next()
                .expect("an emulation without successful points has a failure")
                .context("emulation produced no successful points");
            return Err(RunError::Failed(error));
        }

        chart::render(
            options,
            &series,
            source_bytes,
            (
                self.config.emulation.width.get(),
                self.config.emulation.height.get(),
            ),
        )
        .context("cannot render emulation chart")?;
        diagnostics.emulation(&request.input, &jobs, options, succeeded, failures.len());

        if failures.is_empty() {
            Ok(())
        } else {
            let failed = failures.len();
            let first = failures.into_iter().next().expect("failures is not empty");
            Err(RunError::Failed(
                first.context(format!("{failed} emulation point(s) failed")),
            ))
        }
    }
}
