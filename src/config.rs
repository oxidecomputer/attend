//! Configuration file loading for attend.
//!
//! Config is loaded from two sources and merged:
//! - **Global**: `~/.config/attend/config.toml`
//! - **Hierarchical**: walk from `cwd` upward, collecting `.attend/config.toml`
//!   at each directory level (closer files take precedence; arrays are concatenated).

use std::path::Path;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

use crate::narrate::transcribe::Engine;

/// Attend configuration.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Additional directories to include beyond the project root.
    /// Files in these directories will not be filtered out of dictation/editor context.
    #[serde(default)]
    pub include_dirs: Vec<Utf8PathBuf>,
    /// Transcription engine (`parakeet` or `whisper`).
    pub engine: Option<Engine>,
    /// Custom model path for the transcription engine.
    pub model: Option<Utf8PathBuf>,
    /// Seconds of silence before splitting a recording segment (default 5.0; 0 to disable).
    pub silence_duration: Option<f64>,
    /// How long to keep archived narrations (e.g. `"7d"`, `"24h"`).
    /// Set to `"forever"` to disable automatic cleanup. Defaults to `"7d"`.
    pub archive_retention: Option<String>,
}

impl Config {
    /// Load configuration by walking from `cwd` upward for `.attend/config.toml`
    /// files, then loading the global config. Arrays are concatenated (closer
    /// directories appear later, so they effectively take precedence for ordering).
    ///
    /// Missing files are silently ignored.
    pub fn load(cwd: &Utf8Path) -> Self {
        let mut result = Config::default();

        // Walk upward from cwd (closest first)
        let mut dir = Some(cwd);
        while let Some(d) = dir {
            let cfg_path = d.join(".attend").join("config.toml");
            if let Some(layer) = load_file(cfg_path.as_std_path()) {
                result.merge(layer);
            }
            dir = d.parent();
        }

        // Global config
        if let Some(global_dir) = dirs::config_dir() {
            let cfg_path = global_dir.join("attend").join("config.toml");
            if let Some(layer) = load_file(&cfg_path) {
                result.merge(layer);
            }
        }

        result
    }

    /// Merge another config layer into this one.
    ///
    /// Arrays are concatenated. Scalar fields use "first wins" semantics:
    /// the existing value is kept if already set, otherwise the new value is taken.
    /// Parse `archive_retention` to a [`Duration`], returning `None` for
    /// `"forever"` (cleanup disabled). Defaults to 7 days when unset.
    pub fn retention_duration(&self) -> Option<std::time::Duration> {
        match self.archive_retention.as_deref() {
            Some("forever") => None,
            Some(s) => humantime::parse_duration(s).ok(),
            None => Some(std::time::Duration::from_secs(7 * 24 * 60 * 60)),
        }
    }

    pub fn merge(&mut self, other: Config) {
        self.include_dirs.extend(other.include_dirs);
        if self.engine.is_none() {
            self.engine = other.engine;
        }
        if self.model.is_none() {
            self.model = other.model;
        }
        if self.silence_duration.is_none() {
            self.silence_duration = other.silence_duration;
        }
        if self.archive_retention.is_none() {
            self.archive_retention = other.archive_retention;
        }
    }
}

/// Try to load and parse a single config file. Returns `None` on any failure.
fn load_file(path: &Path) -> Option<Config> {
    let content = std::fs::read_to_string(path).ok()?;
    match toml::from_str(&content) {
        Ok(config) => Some(config),
        Err(e) => {
            tracing::warn!(path = %path.display(), "Failed to parse config: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests;
