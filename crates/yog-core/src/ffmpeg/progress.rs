#[derive(Debug, Clone, Default, PartialEq, Copy)]
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

    #[track_caller]
    fn assert_pending(parser: &mut ProgressParser, line: &str) {
        assert!(parser.push_line(line).is_none());
    }

    fn progress_with_time(value: &str) -> Progress {
        let mut parser = ProgressParser::new();
        parser.push_line(&format!("out_time_us={value}"));
        parser.push_line("speed=N/A");
        parser.push_line("fps=N/A");
        parser.push_line("progress=continue").unwrap()
    }

    #[test]
    fn records_are_delimited_and_missing_fields_do_not_leak() {
        let mut parser = ProgressParser::new();
        assert_pending(&mut parser, "frame=23");
        assert_pending(&mut parser, "fps=24.5");
        assert_pending(&mut parser, "speed=   2x");
        assert_pending(&mut parser, "out_time_us=1250000");
        assert_pending(&mut parser, "out_time_ms=1250000");
        assert_pending(&mut parser, "out_time=00:00:01.250000");
        assert_pending(&mut parser, "stream_0_0_q=18.0");
        assert_pending(&mut parser, "total_size=100");
        assert_pending(&mut parser, "dup_frames=1");
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
        let negative = progress_with_time("-250000");
        assert_eq!(negative.out_time_us, Some(-250_000));
        assert_eq!(negative.speed, None);
        assert_eq!(negative.fps, None);

        let zero = progress_with_time("0");
        assert_eq!(zero.out_time_us, Some(0));
        assert_eq!(zero.speed, None);
        assert_eq!(zero.fps, None);

        let unavailable = progress_with_time("N/A");
        assert_eq!(unavailable.out_time_us, None);
        assert_eq!(unavailable.speed, None);
        assert_eq!(unavailable.fps, None);
    }
}
