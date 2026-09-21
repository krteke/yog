use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, de};
use std::{
    fs, io,
    num::{NonZeroU32, NonZeroU64, NonZeroUsize},
    path::Path,
    time::Duration,
};
use yog_core::ffmpeg::prediction::PredictionOptions;

#[derive(Debug, Deserialize, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub progress_tick_interval_ms: NonZeroU64,
    pub prediction: Prediction,
    pub emulation: Emulation,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(default, deny_unknown_fields)]
pub struct Prediction {
    pub samples: NonZeroUsize,
    #[serde(deserialize_with = "deserialize_duration")]
    pub sample_sec: Duration,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(default, deny_unknown_fields)]
pub struct Emulation {
    pub width: NonZeroU32,
    pub height: NonZeroU32,
}

impl From<Prediction> for PredictionOptions {
    fn from(value: Prediction) -> Self {
        PredictionOptions {
            samples: value.samples,
            sample_duration: value.sample_sec,
        }
    }
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let seconds = f64::deserialize(deserializer)?;

    if seconds <= 0.0 {
        return Err(de::Error::custom("seconds must be greater than 0"));
    }

    Duration::try_from_secs_f64(seconds).map_err(de::Error::custom)
}

impl Default for Prediction {
    fn default() -> Self {
        Self {
            samples: NonZeroUsize::new(5).unwrap(),
            sample_sec: Duration::from_secs_f64(2.0),
        }
    }
}

impl Default for Emulation {
    fn default() -> Self {
        Self {
            width: NonZeroU32::new(1280).unwrap(),
            height: NonZeroU32::new(720).unwrap(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            progress_tick_interval_ms: NonZeroU64::new(100).unwrap(),
            prediction: Prediction::default(),
            emulation: Emulation::default(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let default_path;
        let config = match path {
            Some(path) => path,
            None => {
                let Some(directory) = dirs::config_dir() else {
                    return Ok(Self::default());
                };
                default_path = directory.join("yog").join("config.toml");
                &default_path
            }
        };

        let content = match fs::read_to_string(config) {
            Err(error) if path.is_none() && error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            result => result.with_context(|| format!("cannot read config {}", config.display()))?,
        };

        let config: Self = toml::from_str(&content)
            .with_context(|| format!("invalid config {}", config.display()))?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_keeps_defaults_and_preserves_explicit_zero_ticks() {
        let config: Config = toml::from_str("progress_tick_interval_ms = 1").unwrap();
        assert_eq!(config.progress_tick_interval_ms.get(), 1);
        assert_eq!(config.prediction.samples.get(), 5);
        assert_eq!(config.prediction.sample_sec, Duration::from_secs_f64(2.0));
        assert_eq!(config.emulation.width.get(), 1280);
        assert_eq!(config.emulation.height.get(), 720);
    }

    #[test]
    fn emulation_dimensions_must_be_positive() {
        assert!(toml::from_str::<Config>("[emulation]\nwidth = 0").is_err());
        assert!(toml::from_str::<Config>("[emulation]\nheight = 0").is_err());
    }
}
