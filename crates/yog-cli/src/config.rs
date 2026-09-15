use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, de};
use std::{
    fs, io,
    num::{NonZeroU64, NonZeroUsize},
    path::Path,
    sync::OnceLock,
    time::Duration,
};
use yog_core::ffmpeg::prediction::PredictionOptions;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub progress_tick_interval_ms: NonZeroU64,
    pub prediction: Prediction,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(default, deny_unknown_fields)]
pub struct Prediction {
    pub samples: NonZeroUsize,
    #[serde(deserialize_with = "deserialize_duration")]
    pub sample_sec: Duration,
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

impl Default for Config {
    fn default() -> Self {
        Self {
            progress_tick_interval_ms: NonZeroU64::new(100).unwrap(),
            prediction: Prediction::default(),
        }
    }
}

impl Config {
    fn load(path: Option<&Path>) -> Result<Self> {
        let config = match path {
            Some(path) => path,
            None => match dirs::config_dir() {
                Some(dir) => &dir.join("yog").join("config.toml"),
                None => return Ok(Self::default()),
            },
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

pub fn init(path: Option<&Path>) -> Result<()> {
    CONFIG
        .set(Config::load(path)?)
        .expect("configuration was already initialized");

    Ok(())
}

pub fn get() -> &'static Config {
    CONFIG.get().expect("configuration is not initialized")
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
    }
}
