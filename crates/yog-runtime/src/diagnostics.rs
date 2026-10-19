use std::{
    fmt::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use yog_core::ffmpeg::{
    prediction::Prediction,
    vmaf::{VmafOptions, VmafScore},
};

use crate::{
    emulate::{EmulationJob, EmulationOptions},
    error::RunError,
    event::{EventSink, RecoveryInfo, RunEvent, SkipReason, TaskResult},
};

pub struct Diagnostics<'a> {
    verbose: bool,
    terminal_output: bool,
    stderr_logged: AtomicBool,
    events: &'a EventSink<'a>,
}

impl<'a> Diagnostics<'a> {
    pub fn new(verbose: bool, terminal_output: bool, events: &'a EventSink<'a>) -> Self {
        Self {
            verbose,
            terminal_output,
            stderr_logged: AtomicBool::new(false),
            events,
        }
    }

    pub fn event(&self, event: RunEvent) {
        (self.events)(event);
    }

    pub fn ffmpeg(&self, bytes: &[u8]) -> bool {
        let message = String::from_utf8_lossy(bytes);
        if message.is_empty() || !log::log_enabled!(log::Level::Debug) {
            return false;
        }

        self.stderr_logged.store(true, Ordering::Relaxed);
        log::debug!(target: "yog::ffmpeg", "{message}");
        true
    }

    pub fn begin_task(&self) {
        self.stderr_logged.store(false, Ordering::Relaxed);
    }

    pub fn predict(&self, input: &Path, prediction: &Prediction) {
        if !self.terminal_output {
            return;
        }

        let mut report = String::new();
        if !self.verbose {
            write!(
                report,
                "prediction: {} | speed {:.3}x | time {:.1}s | size {:.1}MiB ({:.1}% of input) | VMAF {:.3}",
                input.display(),
                prediction.speed.value,
                prediction.transcode_seconds.value,
                prediction.output_bytes.value as f64 / 1_048_576.0,
                prediction.output_bytes.value as f64 / prediction.source_bytes as f64 * 100.0,
                prediction.quality.vmaf.value,
            )
            .unwrap();
            if let Some(ssim) = prediction.quality.ssim {
                write!(report, " | SSIM {:.6}", ssim.value).unwrap();
            }
            if let Some(psnr) = prediction.quality.psnr_y_db {
                write!(report, " | PSNR-Y {:.3}dB", psnr.value).unwrap();
            }
            println!("{report}");
            return;
        }

        writeln!(report, "prediction: {}", input.display()).unwrap();
        writeln!(
            report,
            "  source: {:.3}s, {:.1}MiB",
            prediction.source_duration_seconds,
            prediction.source_bytes as f64 / 1_048_576.0,
        )
        .unwrap();
        writeln!(
            report,
            "  samples: {}, {:.3}s total",
            prediction.samples.len(),
            prediction.sampled_seconds,
        )
        .unwrap();
        writeln!(
            report,
            "  sampled speed: {:.3}x (sample range {:.3}x..{:.3}x)",
            prediction.speed.value, prediction.speed.low, prediction.speed.high,
        )
        .unwrap();
        writeln!(
            report,
            "  sample-extrapolated time: {:.1}s ({:.1}s..{:.1}s)",
            prediction.transcode_seconds.value,
            prediction.transcode_seconds.low,
            prediction.transcode_seconds.high,
        )
        .unwrap();
        writeln!(
            report,
            "  estimated size: {:.1}MiB (sample range {:.1}MiB..{:.1}MiB, {:.1}% of input)",
            prediction.output_bytes.value as f64 / 1_048_576.0,
            prediction.output_bytes.low as f64 / 1_048_576.0,
            prediction.output_bytes.high as f64 / 1_048_576.0,
            prediction.output_bytes.value as f64 / prediction.source_bytes as f64 * 100.0,
        )
        .unwrap();
        writeln!(
            report,
            "  VMAF: {:.3} ({:.3}..{:.3}, stream #{}, {} frames)",
            prediction.quality.vmaf.value,
            prediction.quality.vmaf.low,
            prediction.quality.vmaf.high,
            prediction.quality.source_stream_index,
            prediction.quality.frames,
        )
        .unwrap();
        if let Some(ssim) = prediction.quality.ssim {
            writeln!(
                report,
                "  SSIM: {:.6} ({:.6}..{:.6})",
                ssim.value, ssim.low, ssim.high,
            )
            .unwrap();
        }
        if let Some(psnr) = prediction.quality.psnr_y_db {
            writeln!(
                report,
                "  PSNR-Y: {:.3}dB ({:.3}dB..{:.3}dB)",
                psnr.value, psnr.low, psnr.high,
            )
            .unwrap();
        }
        for (index, sample) in prediction.samples.iter().enumerate() {
            write!(
                report,
                "  sample #{}: start {:.3}s, length {:.3}s, encode {:.3}s, speed {:.3}x, payload {:.1}MiB, {} frames, VMAF {:.3}",
                index + 1,
                sample.start_seconds,
                sample.duration_seconds,
                sample.encode_seconds,
                sample.speed,
                sample.timed_payload_bytes as f64 / 1_048_576.0,
                sample.scored_frames,
                sample.vmaf,
            )
            .unwrap();
            if let Some(ssim) = sample.ssim {
                write!(report, ", SSIM {ssim:.6}").unwrap();
            }
            if let Some(psnr) = sample.psnr_y_db {
                write!(report, ", PSNR-Y {psnr:.3}dB").unwrap();
            }
            report.push('\n');
        }
        report.pop();
        println!("{report}");
    }

    pub fn emulation(
        &self,
        input: &Path,
        jobs: &[EmulationJob],
        options: &EmulationOptions,
        succeeded: usize,
        failed: usize,
    ) {
        if !self.terminal_output {
            return;
        }

        if !self.verbose {
            let mut report = format!(
                "emulation: {} | candidates {} | points {succeeded} succeeded, {failed} failed",
                input.display(),
                jobs.len(),
            );
            append_emulation_outputs(&mut report, options);
            println!("{report}");
            return;
        }

        for (index, job) in jobs.iter().enumerate() {
            let label = if job.is_candidate() {
                format!("{} | ", job.label())
            } else {
                String::new()
            };
            let mut report = format!(
                "emulation: {} | {label}{} {}",
                input.display(),
                job.parameter(),
                format_qualities(job.qualities()),
            );
            if index == 0 {
                append_emulation_outputs(&mut report, options);
            }
            println!("{report}");
        }
    }

    pub fn vmaf(&self, output: &Path, score: VmafScore, options: VmafOptions) {
        if !self.terminal_output {
            return;
        }

        let mode = options
            .n_subsample
            .map(|value| format!("n_subsample={value}"))
            .unwrap_or_else(|| "full".to_owned());
        println!(
            "vmaf: {} | score {:.3} | {mode}",
            output.display(),
            score.value,
        );
    }

    pub fn vmaf_warning(&self, error: &anyhow::Error, stderr_logged: bool) {
        self.event(RunEvent::Warning {
            message: format!("vmaf: {error:#}"),
        });
        if !self.terminal_output {
            return;
        }

        eprintln!("warning: vmaf: {error:#}");
        if let Some(error) = Self::program_error(error) {
            Self::print_program_stderr(error, stderr_logged);
        }
    }

    pub fn vmaf_skipped(&self, output: &Path) {
        if self.terminal_output {
            println!("vmaf: {} | manually skipped", output.display());
        }
    }

    pub fn vmaf_cancelled(&self, output: &Path) {
        self.event(RunEvent::Warning {
            message: format!("vmaf cancelled for {}", output.display()),
        });
        if self.terminal_output {
            eprintln!("warning: vmaf cancelled for {}", output.display());
        }
    }

    pub fn error(&self, error: &anyhow::Error) {
        self.event(RunEvent::Error {
            message: format!("{error:#}"),
        });
        if !self.terminal_output {
            return;
        }

        eprintln!("{error:#}");

        self.print_error_details(error);
    }

    pub fn task_error(&self, input: Option<&Path>, failure: &RunError) {
        if !self.terminal_output {
            return;
        }

        let result = failure.result();
        if let Some(input) = input {
            eprintln!(
                "failed: {}: {result}: {}",
                input.display(),
                result.message().unwrap()
            );
        } else {
            eprintln!("{result}: {}", result.message().unwrap());
        }
        let RunError::Failed { error, .. } = failure else {
            unreachable!("task_error expects a failure")
        };
        self.print_error_details(error);
    }

    pub fn batch_summary(&self, batch: &crate::BatchOutcome) {
        if !self.terminal_output {
            return;
        }
        let mut summary = format!(
            "batch summary: total {} | succeeded {} | failed {} | skipped {} | reused {} | forced {}",
            batch.total,
            batch.succeeded,
            batch.failures.len(),
            batch.skipped,
            batch.reused,
            batch.forced,
        );
        if batch.manual_skipped > 0 {
            write!(summary, " | manual {}", batch.manual_skipped).unwrap();
        }
        if batch.existing_skipped > 0 {
            write!(summary, " | existing {}", batch.existing_skipped).unwrap();
        }
        if batch.cancelled || batch.state_error.is_some() || batch.recovery_error.is_some() {
            write!(
                summary,
                " | not processed {}",
                batch.total - batch.succeeded - batch.failures.len() - batch.skipped - batch.reused
            )
            .unwrap();
        }
        if batch.cancelled {
            summary.push_str(" | cancelled");
        }
        if batch.state_error.is_some() {
            summary.push_str(" | state error");
        }
        if batch.recovery_error.is_some() {
            summary.push_str(" | recovery error");
        }
        println!("{summary}");
    }

    pub fn skipped_input(&self, input: &Path, failure: &RunError) {
        if !self.terminal_output {
            return;
        }
        let result = failure.result();
        let RunError::Skipped { reason, error } = failure else {
            unreachable!("skipped_input expects a skip")
        };
        if matches!(reason, SkipReason::Manual | SkipReason::ExistingOutput) {
            println!("skipped: {}: {result}", input.display());
            return;
        }
        eprintln!(
            "warning: skipping {}: {result}: {}",
            input.display(),
            result.message().unwrap()
        );
        if let Some(error) = Self::program_error(error) {
            Self::print_program_stderr(error, false);
        }
    }

    pub fn recovered_task(&self, input: &Path, result: &TaskResult, info: &RecoveryInfo) {
        if self.terminal_output && self.verbose {
            println!(
                "recovery: {}: {result}{}",
                input.display(),
                if info.forced { " (forced)" } else { "" }
            );
        }
    }

    pub fn warning(&self, message: &str) {
        self.event(RunEvent::Warning {
            message: message.into(),
        });
        if self.terminal_output {
            eprintln!("warning: {message}");
        }
    }

    pub fn recovery(&self, path: &Path, reused: usize, forced: usize) {
        if self.terminal_output {
            println!(
                "recovery: {} | reused {reused} | forced {forced}",
                path.display()
            );
        }
        if forced > 0 {
            self.warning("Recovery reused records after accepting differences; completed outputs remain at their recorded paths");
        }
    }

    pub fn complete(&self, output: &Path) {
        if self.terminal_output {
            println!("complete: {}", output.display());
        }
    }

    pub fn verify_warning(&self, warning: &str) {
        self.event(RunEvent::Warning {
            message: format!("verify: {warning}"),
        });
        if self.terminal_output {
            eprintln!("warning: verify: {warning}");
        }
    }

    pub fn report_error(&self, path: Option<&Path>, error: &std::io::Error) {
        let message = match path {
            Some(path) => format!("cannot write report {}: {error}", path.display()),
            None => format!("cannot write report: {error}"),
        };
        self.event(RunEvent::Warning { message });
        if !self.terminal_output {
            return;
        }

        match path {
            Some(path) => eprintln!("warning: cannot write report {}: {error}", path.display()),
            None => eprintln!("warning: cannot write report: {error}"),
        }
    }

    pub fn cancelled(&self) {
        if self.terminal_output {
            println!("cancelled");
        }
    }

    fn print_error_details(&self, error: &anyhow::Error) {
        let Some(error) = Self::program_error(error) else {
            return;
        };

        Self::print_program_stderr(error, self.stderr_logged.load(Ordering::Relaxed));
    }

    fn program_error(error: &anyhow::Error) -> Option<&yog_core::error::Error> {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<yog_core::error::Error>())
    }

    fn print_program_stderr(error: &yog_core::error::Error, stderr_logged: bool) {
        if error.stderr.is_empty() || stderr_logged {
            return;
        }
        eprint!("{}", String::from_utf8_lossy(&error.stderr));
        if !error.stderr.ends_with(b"\n") {
            eprintln!();
        }
    }
}

fn append_emulation_outputs(report: &mut String, options: &EmulationOptions) {
    if let Some(path) = &options.png {
        write!(report, " | PNG {}", path.display()).unwrap();
    }
    if let Some(path) = &options.svg {
        write!(report, " | SVG {}", path.display()).unwrap();
    }
}

fn format_qualities(qualities: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut run: Option<(u8, u8)> = None;

    for &quality in qualities {
        run = Some(match run {
            Some((start, end)) if quality == end.saturating_add(1) => (start, quality),
            Some((start, end)) => {
                parts.push(format_run(start, end));
                (quality, quality)
            }
            None => (quality, quality),
        });
    }
    if let Some((start, end)) = run {
        parts.push(format_run(start, end));
    }

    parts.join(",")
}

fn format_run(start: u8, end: u8) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

#[cfg(test)]
mod tests {
    use super::format_qualities;

    #[test]
    fn quality_points_collapse_into_ascending_runs() {
        assert_eq!(format_qualities(&[20]), "20");
        assert_eq!(format_qualities(&[20, 21, 22]), "20-22");
        assert_eq!(format_qualities(&[18, 19, 20, 25]), "18-20,25");
        assert_eq!(format_qualities(&[0, 255]), "0,255");
        assert_eq!(format_qualities(&[254, 255]), "254-255");
        assert_eq!(format_qualities(&[]), "");
    }
}
