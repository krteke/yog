use std::time::Duration;

/// One complete machine-progress record. `finished` means `progress=end`,
/// which does not establish that the process exited successfully.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub frame: Option<u64>,
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    pub total_size: Option<u64>,
    /// Signed microseconds; negative timestamps remain representable.
    pub out_time_us: Option<i64>,
    pub dup_frames: Option<u64>,
    pub drop_frames: Option<u64>,
    pub finished: bool,
}

/// Feed lines without requiring a known input duration or frame rate.
#[derive(Debug, Default)]
pub struct ProgressParser {
    current: Progress,
    legacy_time_us: Option<i64>,
    timestamp_us: Option<i64>,
}

impl ProgressParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_line(&mut self, line: &str) -> Option<Progress> {
        let (key, value) = line.trim().split_once('=')?;
        match key {
            "frame" => self.current.frame = value.parse().ok(),
            "fps" => self.current.fps = number(value),
            "speed" => self.current.speed = number(value.strip_suffix('x').unwrap_or(value)),
            "total_size" => self.current.total_size = value.parse().ok(),
            "out_time_us" => self.current.out_time_us = value.parse().ok(),
            // This field is also in microseconds despite its name.
            "out_time_ms" => self.legacy_time_us = value.parse().ok(),
            "out_time" => self.timestamp_us = timestamp_us(value),
            "dup_frames" => self.current.dup_frames = value.parse().ok(),
            "drop_frames" => self.current.drop_frames = value.parse().ok(),
            "progress" => {
                let mut record = std::mem::take(&mut self.current);
                record.out_time_us = record
                    .out_time_us
                    .or(self.legacy_time_us.take())
                    .or(self.timestamp_us.take());
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

fn timestamp_us(value: &str) -> Option<i64> {
    let (negative, value) = match value.strip_prefix('-') {
        Some(value) => (true, value),
        None => (false, value),
    };
    let mut parts = value.split(':');
    let hours = parts.next()?.parse::<u64>().ok()?;
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds = number(parts.next()?)?;
    if parts.next().is_some() || minutes >= 60 || !(0.0..60.0).contains(&seconds) {
        return None;
    }
    let whole_seconds = hours.checked_mul(3600)?.checked_add(minutes * 60)?;
    let micros = Duration::from_secs(whole_seconds)
        .checked_add(Duration::try_from_secs_f64(seconds).ok()?)?
        .as_micros();
    let signed = i128::try_from(micros).ok()?;
    i64::try_from(if negative { -signed } else { signed }).ok()
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
            "speed=2.0x",
            "out_time_us=1250000",
            "out_time_ms=2500000",
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
    fn unavailable_and_signed_times_use_valid_protocol_alternatives() {
        let mut parser = ProgressParser::new();
        for line in [
            "out_time_us=N/A",
            "out_time_ms=-250000",
            "speed=NaNx",
            "fps=N/A",
        ] {
            parser.push_line(line);
        }
        let progress = parser.push_line("progress=continue").unwrap();
        assert_eq!(progress.out_time_us, Some(-250_000));
        assert_eq!(progress.speed, None);
        parser.push_line("out_time=-00:00:00.125000");
        assert_eq!(
            parser.push_line("progress=end").unwrap().out_time_us,
            Some(-125_000)
        );
    }

    #[test]
    fn malformed_and_overflowing_timestamps_never_panic() {
        for timestamp in [
            "NaN:00:01",
            "1e300:00:00",
            "18446744073709551615:00:00",
            "01:60:00",
            "01:00:60",
            "00:00:inf",
            "00:00:01:02",
        ] {
            assert_eq!(timestamp_us(timestamp), None, "{timestamp}");
        }
        assert_eq!(timestamp_us("123:45:06.123456"), Some(445_506_123_456));
        let mut parser = ProgressParser::new();
        parser.push_line("frame=10");
        assert!(parser.push_line("progress=invalid").is_none());
        assert_eq!(parser.push_line("progress=end").unwrap().frame, None);
    }
}
