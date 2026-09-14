use std::{
    fmt::Write as _,
    io::{self, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

use tokio::task::{JoinHandle, spawn_blocking};
use tokio_util::sync::CancellationToken;
use yog_core::ffmpeg::{
    prediction::Prediction,
    vmaf::{VmafOptions, VmafScore},
};

use crate::config;

pub struct Diagnostics {
    sender: Option<Sender<Vec<u8>>>,
    worker: Option<JoinHandle<()>>,
    stopping: CancellationToken,
    verbose: bool,
    streamed: AtomicBool,
}

impl Diagnostics {
    pub fn new(verbose: bool, cancelled: CancellationToken) -> Self {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(if verbose {
            "debug"
        } else {
            "warn"
        }))
        .format_timestamp(None)
        .format_target(false)
        .init();

        let retry_interval =
            Duration::from_millis(config::get().diagnostics_retry_interval_ms.get());

        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        let stopping = CancellationToken::new();
        let stop = stopping.clone();

        let worker = spawn_blocking(move || {
            for bytes in receiver {
                let mut remaining = bytes.as_slice();
                while !remaining.is_empty() {
                    let result = io::stderr().lock().write(remaining);
                    match result {
                        Ok(0) => return,
                        Ok(count) => remaining = &remaining[count..],
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            if stop.is_cancelled() || cancelled.is_cancelled() {
                                return;
                            }
                            thread::sleep(retry_interval);
                        }
                        Err(_) => return,
                    }
                }
            }
        });

        Self {
            sender: Some(sender),
            worker: Some(worker),
            stopping,
            verbose,
            streamed: AtomicBool::new(false),
        }
    }

    pub fn write(&self, bytes: &[u8]) {
        let _ = self.sender.as_ref().unwrap().send(bytes.to_owned());
    }

    pub fn ffmpeg(&self, bytes: &[u8]) {
        if self.verbose {
            self.streamed.store(true, Ordering::Relaxed);
            self.write(bytes);
        }
    }

    pub fn begin_task(&self) {
        self.streamed.store(false, Ordering::Relaxed);
    }

    pub fn predict(&self, input: &Path, prediction: &Prediction) {
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
            report.push('\n');
            self.write(report.as_bytes());
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
        self.write(report.as_bytes());
    }

    pub fn vmaf(&self, output: &Path, score: VmafScore, options: VmafOptions) {
        let mode = options
            .n_subsample
            .map(|value| format!("n_subsample={value}"))
            .unwrap_or_else(|| "full".to_owned());
        self.write(
            format!(
                "vmaf: {} | score {:.3} | {mode}\n",
                output.display(),
                score.value,
            )
            .as_bytes(),
        );
    }

    pub fn vmaf_warning(&self, error: &anyhow::Error, stderr_streamed: bool) {
        self.write(format!("warning: vmaf: {error:#}\n").as_bytes());
        if let Some(error) = Self::program_error(error) {
            self.write_program_stderr(error, stderr_streamed);
        }
    }

    pub fn error(&self, error: &anyhow::Error) {
        self.write(format!("{error:#}\n").as_bytes());

        self.write_error_details(error);
    }

    pub fn task_error(&self, input: &Path, error: &anyhow::Error) {
        self.write(format!("failed: {}: {error:#}\n", input.display()).as_bytes());

        self.write_error_details(error);
    }

    pub fn batch_summary(&self, total: usize, succeeded: usize, failed: usize, cancelled: bool) {
        let mut summary =
            format!("batch summary: total {total} | succeeded {succeeded} | failed {failed}");
        if cancelled {
            write!(
                summary,
                " | not processed {} | cancelled",
                total - succeeded - failed,
            )
            .unwrap();
        }
        summary.push('\n');
        self.write(summary.as_bytes());
    }

    fn write_error_details(&self, error: &anyhow::Error) {
        let Some(error) = Self::program_error(error) else {
            return;
        };

        self.write_program_stderr(error, self.streamed.load(Ordering::Relaxed));
        if matches!(error.reason, yog_core::error::Failure::TimedOut) {
            self.stopping.cancel();
        }
    }

    fn program_error(error: &anyhow::Error) -> Option<&yog_core::error::Error> {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<yog_core::error::Error>())
    }

    fn write_program_stderr(&self, error: &yog_core::error::Error, stderr_streamed: bool) {
        if !error.stderr.is_empty() {
            if !stderr_streamed {
                self.write(&error.stderr);
            }
            if !error.stderr.ends_with(b"\n") {
                self.write(b"\n");
            }
        }
    }

    pub async fn finish(mut self) {
        self.sender.take();
        if let Err(error) = self.worker.take().unwrap().await {
            std::panic::resume_unwind(error.into_panic());
        }
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.stopping.cancel();
    }
}
