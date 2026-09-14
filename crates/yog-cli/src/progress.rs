use crate::config;
use indicatif::{ProgressBar, ProgressStyle};
use std::{fmt::Write, time::Duration};
use yog_core::ffmpeg::progress::Progress;

pub struct Display {
    bar: ProgressBar,
}

impl Display {
    pub fn new(verbose: bool) -> Self {
        Self::with_template(verbose, "{spinner} {msg} [{elapsed_precise}]")
    }

    pub fn predicting(verbose: bool) -> Self {
        Self::with_template(verbose, "{spinner} Predicting...")
    }

    fn with_template(verbose: bool, template: &str) -> Self {
        let bar = if verbose {
            ProgressBar::hidden()
        } else {
            ProgressBar::new_spinner()
        };
        bar.set_style(ProgressStyle::with_template(template).unwrap());

        if !bar.is_hidden() {
            bar.enable_steady_tick(Duration::from_millis(
                config::get().progress_tick_interval_ms.get(),
            ));
        }
        Self { bar }
    }

    pub fn start(&self, duration: Option<Duration>) {
        let duration = duration
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
    fn time_based_progress_clamps_signed_times_and_overruns_without_finishing() {
        let display = Display::new(true);
        display.start(Some(Duration::from_millis(1500)));
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
