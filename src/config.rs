//! Configuration file loading for attend.
//!
//! Config is loaded from two sources and merged:
//! - **Global**: `~/.config/attend/config.toml`
//! - **Hierarchical**: walk from `cwd` upward, collecting `.attend/config.toml`
//!   at each directory level (closer files take precedence; arrays are concatenated).

use std::fmt;
use std::path::Path;

use camino::{Utf8Path, Utf8PathBuf};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};

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
    /// Duration of silence before splitting a recording segment (e.g. `"5s"`, `"500ms"`).
    /// Defaults to `"5s"`. Set to `"0s"` to disable.
    #[serde(default, deserialize_with = "deserialize_silence_duration")]
    pub silence_duration: Option<String>,
    /// How long to keep archived narrations (e.g. `"7d"`, `"24h"`).
    /// Set to `"forever"` to disable automatic cleanup. Defaults to `"7d"`.
    pub archive_retention: Option<String>,
    /// Whether to capture clipboard changes (text and images). Defaults to true.
    #[serde(default)]
    pub clipboard_capture: Option<bool>,
    /// How long the persistent daemon idles before auto-exiting (e.g. `"5m"`, `"10m"`).
    /// Set to `"forever"` to never auto-exit. Defaults to `"5m"`.
    pub daemon_idle_timeout: Option<String>,
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
        if let Some(global_dir) = crate::util::xdg_config_home() {
            let cfg_path = global_dir.join("attend").join("config.toml");
            if let Some(layer) = load_file(cfg_path.as_std_path()) {
                result.merge(layer);
            }
        }

        result
    }

    /// Parse `archive_retention` to a [`Duration`], returning `None` for
    /// `"forever"` (cleanup disabled). Defaults to 7 days when unset or
    /// when the value cannot be parsed (with a warning).
    pub fn retention_duration(&self) -> Option<std::time::Duration> {
        parse_optional_duration(
            self.archive_retention.as_deref(),
            "archive_retention",
            std::time::Duration::from_secs(7 * 24 * 60 * 60),
        )
    }

    /// Parse `daemon_idle_timeout` to a [`Duration`], returning `None` for
    /// `"forever"` (never auto-exit). Defaults to 5 minutes when unset or
    /// when the value cannot be parsed (with a warning).
    pub fn idle_timeout(&self) -> Option<std::time::Duration> {
        parse_optional_duration(
            self.daemon_idle_timeout.as_deref(),
            "daemon_idle_timeout",
            std::time::Duration::from_secs(5 * 60),
        )
    }

    /// Parse `silence_duration` to a [`Duration`]. Returns `None` when the
    /// value is `"0s"` (silence splitting disabled). Defaults to 5 seconds
    /// when unset or when the value cannot be parsed (with a warning).
    pub fn silence_duration(&self) -> Option<std::time::Duration> {
        match self.silence_duration.as_deref() {
            Some("0s") | Some("0ms") => None,
            value => parse_optional_duration(
                value,
                "silence_duration",
                std::time::Duration::from_secs(5),
            ),
        }
    }

    /// Merge another config layer into this one.
    ///
    /// Arrays are concatenated. Scalar fields use "first wins" semantics:
    /// the existing value is kept if already set, otherwise the new value is taken.
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
        if self.clipboard_capture.is_none() {
            self.clipboard_capture = other.clipboard_capture;
        }
        if self.daemon_idle_timeout.is_none() {
            self.daemon_idle_timeout = other.daemon_idle_timeout;
        }
    }
}

/// Parse a human-readable duration string, returning `None` for `"forever"`.
/// Falls back to `default` (with a warning) when the value is unparseable.
fn parse_optional_duration(
    value: Option<&str>,
    field_name: &str,
    default: std::time::Duration,
) -> Option<std::time::Duration> {
    match value {
        Some("forever") => None,
        Some(s) => match humantime::parse_duration(s) {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::warn!(value = s, "invalid {field_name}, using default: {e}");
                Some(default)
            }
        },
        None => Some(default),
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

/// Deserialize `silence_duration` from either a humantime string (`"5s"`) or a
/// legacy float (seconds, e.g. `2.5`). Floats are converted via milliseconds
/// to avoid precision issues with fractional seconds.
fn deserialize_silence_duration<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct SilenceDurationVisitor;

    impl<'de> Visitor<'de> for SilenceDurationVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a duration string (e.g. \"5s\") or a number of seconds")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            if v < 0.0 {
                return Err(E::custom("silence_duration must not be negative"));
            }
            let millis = (v * 1000.0) as u64;
            Ok(Some(format!("{millis}ms")))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            if v < 0 {
                return Err(E::custom("silence_duration must not be negative"));
            }
            Ok(Some(format!("{v}s")))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(format!("{v}s")))
        }
    }

    deserializer.deserialize_any(SilenceDurationVisitor)
}

#[cfg(test)]
mod tests;
