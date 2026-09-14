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
use yog_core::ffmpeg::prediction::Prediction;

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

    pub fn error(&self, error: &anyhow::Error) {
        self.write(format!("{error:#}\n").as_bytes());

        let Some(err) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<yog_core::error::Error>())
        else {
            return;
        };

        if !err.stderr.is_empty() {
            if !self.streamed.load(Ordering::Relaxed) {
                self.write(&err.stderr);
            }
            if !err.stderr.ends_with(b"\n") {
                self.write(b"\n");
            }
        }
        if matches!(err.reason, yog_core::error::Failure::TimedOut) {
            self.stopping.cancel();
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
