use anyhow::{Context, Result};
use serde::Deserialize;
use std::{fs, path::Path, sync::OnceLock};

static CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub progress_tick_interval_ms: u64,
    pub diagnostics_retry_interval_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            progress_tick_interval_ms: 100,
            diagnostics_retry_interval_ms: 10,
        }
    }
}

impl Config {
    fn load(path: Option<&Path>) -> Result<Self> {
        let config = match path {
            Some(path) => path.to_owned(),
            None => match dirs::config_dir() {
                Some(dir) => dir.join("yog").join("config.toml"),
                None => return Ok(Self::default()),
            },
        };

        let Ok(content) = fs::read_to_string(&config) else {
            return Ok(Self::default());
        };

        toml::from_str(&content).with_context(|| format!("invalid config {}", config.display()))
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
        let config: Config = toml::from_str("progress_tick_interval_ms = 0").unwrap();
        assert_eq!(config.progress_tick_interval_ms, 0);
        assert_eq!(config.diagnostics_retry_interval_ms, 10);

        let config: Config = toml::from_str("diagnostics_retry_interval_ms = 25").unwrap();
        assert_eq!(config.progress_tick_interval_ms, 100);
        assert_eq!(config.diagnostics_retry_interval_ms, 25);
    }
}
