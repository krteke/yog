use crate::config;
use indicatif::{ProgressBar, ProgressStyle};
use std::{fmt::Write, time::Duration};
use yog_core::ffmpeg::progress::Progress;

pub struct Display {
    bar: ProgressBar,
}

impl Display {
    pub fn new(verbose: bool) -> Self {
        let bar = if verbose {
            ProgressBar::hidden()
        } else {
            ProgressBar::new_spinner()
        };
        bar.set_style(ProgressStyle::with_template("{spinner} {msg} [{elapsed_precise}]").unwrap());
        bar.set_message("probing input file");
        if !bar.is_hidden() {
            bar.enable_steady_tick(Duration::from_millis(
                config::get().progress_tick_interval_ms,
            ));
        }
        Self { bar }
    }

    pub fn start(&self, duration: Option<&str>) {
        let duration = duration
            .and_then(|value| value.parse::<f64>().ok())
            .and_then(|value| Duration::try_from_secs_f64(value).ok())
            .and_then(|value| u64::try_from(value.as_micros()).ok())
            .filter(|value| *value > 0);
        if let Some(duration) = duration {
            self.bar.set_length(duration);
            self.bar.set_style(
                ProgressStyle::with_template(
                    "{spinner} [{wide_bar}] {percent}% {msg} elapsed: {elapsed_precise} eta: {eta_precise}",
                )
                .unwrap(),
            );
        }
        self.bar.reset_elapsed();
        self.bar.set_message("processing");
    }

    pub fn update(&self, progress: Progress) {
        if let Some(time) = progress.out_time_us {
            let position = time.max(0) as u64;
            self.bar
                .set_position(self.bar.length().map_or(position, |len| position.min(len)));
        }
        let mut message = String::new();
        if let Some(time) = progress.out_time_us {
            write!(message, "processed: {:.1}s  ", time as f64 / 1_000_000.0).unwrap();
        }
        if let Some(frame) = progress.frame {
            write!(message, "frame: {frame}  ").unwrap();
        }
        if let Some(fps) = progress.fps {
            write!(message, "{fps:.1}fps  ").unwrap();
        }
        if let Some(speed) = progress.speed {
            write!(message, "{speed:.2}x  ").unwrap();
        }
        if let Some(size) = progress.total_size {
            write!(message, "{:.1}MiB  ", size as f64 / 1_048_576.0).unwrap();
        }
        self.bar.set_message(message);
    }

    pub fn publishing(&self) {
        self.bar.set_message("publishing");
    }

    pub fn verifying(&self) {
        self.bar.set_style(
            ProgressStyle::with_template("{spinner} {msg} [{elapsed_precise}]").unwrap(),
        );
        self.bar.set_message("verifying output");
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        self.bar.finish_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_invalid_duration_stays_indeterminate_and_end_is_not_success() {
        for duration in [
            None,
            Some("N/A"),
            Some("NaN"),
            Some("inf"),
            Some("-1"),
            Some("0"),
            Some("1e30"),
        ] {
            let display = Display::new(true);
            display.start(duration);
            assert_eq!(display.bar.length(), None, "{duration:?}");
            display.update(Progress {
                out_time_us: Some(500_000),
                finished: true,
                ..Progress::default()
            });
            assert!(!display.bar.is_finished());
            display.publishing();
            assert!(!display.bar.is_finished());
        }
    }

    #[test]
    fn time_based_progress_clamps_signed_times_and_overruns_without_finishing() {
        let display = Display::new(true);
        display.start(Some("1.5"));
        assert_eq!(display.bar.length(), Some(1_500_000));
        for (time, expected) in [(-250_000, 0), (750_000, 750_000), (2_000_000, 1_500_000)] {
            display.update(Progress {
                out_time_us: Some(time),
                ..Progress::default()
            });
            assert_eq!(display.bar.position(), expected);
            assert!(!display.bar.is_finished());
        }
    }
}
