#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub frame: Option<u64>,
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    pub total_size: Option<u64>,
    /// signed microseconds, `N/A` is `None`.
    pub out_time_us: Option<i64>,
    pub dup_frames: Option<u64>,
    pub drop_frames: Option<u64>,
    pub finished: bool,
}

/// Feed lines without requiring a known input duration or frame rate.
#[derive(Debug, Default)]
pub struct ProgressParser {
    current: Progress,
}

impl ProgressParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_line(&mut self, line: &str) -> Option<Progress> {
        let (key, value) = line.trim().split_once('=')?;
        let value = value.trim();
        match key {
            "frame" => self.current.frame = value.parse().ok(),
            "fps" => self.current.fps = number(value),
            "speed" => self.current.speed = number(value.strip_suffix('x').unwrap_or(value)),
            "total_size" => self.current.total_size = value.parse().ok(),
            "out_time_us" => self.current.out_time_us = value.parse().ok(),
            "dup_frames" => self.current.dup_frames = value.parse().ok(),
            "drop_frames" => self.current.drop_frames = value.parse().ok(),
            "progress" => {
                let mut record = std::mem::take(&mut self.current);
                if !matches!(value, "continue" | "end") {
                    return None;
                }
                record.finished = value == "end";
                return Some(record);
            }
            _ => {}
        }
        None
    }
}

fn number(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|n| n.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_are_delimited_and_missing_fields_do_not_leak() {
        let mut parser = ProgressParser::new();
        for line in [
            "frame=23",
            "fps=24.5",
            "speed=   2x",
            "out_time_us=1250000",
            "out_time_ms=1250000",
            "out_time=00:00:01.250000",
            "stream_0_0_q=18.0",
            "total_size=100",
            "dup_frames=1",
        ] {
            assert!(parser.push_line(line).is_none());
        }
        let first = parser.push_line("progress=continue").unwrap();
        assert_eq!(first.out_time_us, Some(1_250_000));
        assert_eq!(first.speed, Some(2.0));
        assert!(!first.finished);
        assert_eq!(
            parser.push_line("progress=end").unwrap(),
            Progress {
                finished: true,
                ..Progress::default()
            }
        );
    }

    #[test]
    fn signed_zero_and_unavailable_times_are_distinct() {
        let mut parser = ProgressParser::new();
        for (value, expected) in [("-250000", Some(-250_000)), ("0", Some(0)), ("N/A", None)] {
            parser.push_line(&format!("out_time_us={value}"));
            parser.push_line("speed=N/A");
            parser.push_line("fps=N/A");
            let progress = parser.push_line("progress=continue").unwrap();
            assert_eq!(progress.out_time_us, expected);
            assert_eq!(progress.speed, None);
            assert_eq!(progress.fps, None);
        }
    }
}
