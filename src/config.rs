//! Configuration file loading for attend.
//!
//! Config is loaded from two sources and merged:
//! - **Global**: `~/.config/attend/config.toml`
//! - **Hierarchical**: walk from `cwd` upward, collecting `.attend/config.toml`
//!   at each directory level (closer files take precedence; arrays are concatenated).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::narrate::transcribe::Engine;

/// Attend configuration.
#[derive(Debug, Default)]
pub struct Config {
    /// Additional directories to include beyond the project root.
    /// Files in these directories will not be filtered out of dictation/editor context.
    pub include_dirs: Vec<PathBuf>,
    /// Transcription engine (`parakeet` or `whisper`).
    pub engine: Option<Engine>,
    /// Custom model path for the transcription engine.
    pub model: Option<PathBuf>,
    /// Seconds of silence before splitting a recording segment (default 5.0; 0 to disable).
    pub silence_duration: Option<f64>,
}

/// Raw TOML representation (may have missing fields).
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    include_dirs: Vec<PathBuf>,
    engine: Option<String>,
    model: Option<PathBuf>,
    silence_duration: Option<f64>,
}

impl Config {
    /// Load configuration by walking from `cwd` upward for `.attend/config.toml`
    /// files, then loading the global config. Arrays are concatenated (closer
    /// directories appear later, so they effectively take precedence for ordering).
    ///
    /// Missing files are silently ignored.
    pub fn load(cwd: &Path) -> Self {
        let mut include_dirs = Vec::new();
        // Scalar fields: "closest wins" (first value found takes precedence).
        let mut engine: Option<Engine> = None;
        let mut model: Option<PathBuf> = None;
        let mut silence_duration: Option<f64> = None;

        // Walk upward from cwd (closest first)
        let mut dir = Some(cwd);
        while let Some(d) = dir {
            let cfg_path = d.join(".attend").join("config.toml");
            if let Some(raw) = load_file(&cfg_path) {
                include_dirs.extend(raw.include_dirs);
                if engine.is_none() {
                    engine = raw.engine.as_deref().and_then(parse_engine);
                }
                if model.is_none() {
                    model = raw.model;
                }
                if silence_duration.is_none() {
                    silence_duration = raw.silence_duration;
                }
            }
            dir = d.parent();
        }

        // Global config
        if let Some(global_dir) = dirs::config_dir() {
            let cfg_path = global_dir.join("attend").join("config.toml");
            if let Some(raw) = load_file(&cfg_path) {
                include_dirs.extend(raw.include_dirs);
                if engine.is_none() {
                    engine = raw.engine.as_deref().and_then(parse_engine);
                }
                if model.is_none() {
                    model = raw.model;
                }
                if silence_duration.is_none() {
                    silence_duration = raw.silence_duration;
                }
            }
        }

        Config {
            include_dirs,
            engine,
            model,
            silence_duration,
        }
    }
}

/// Try to load and parse a single config file. Returns `None` on any failure.
fn load_file(path: &Path) -> Option<RawConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

/// Parse an engine name string into an `Engine` variant.
fn parse_engine(s: &str) -> Option<Engine> {
    match s {
        "parakeet" => Some(Engine::Parakeet),
        "whisper" => Some(Engine::Whisper),
        _ => {
            tracing::warn!(engine = s, "Unknown engine in config, ignoring");
            None
        }
    }
}

#[cfg(test)]
mod tests;
