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
mod tests {
    use super::*;

    #[test]
    fn load_missing_dir() {
        let config = Config::load(Path::new("/nonexistent/path"));
        assert!(config.include_dirs.is_empty());
    }

    #[test]
    fn load_file_missing() {
        assert!(load_file(Path::new("/nonexistent/config.toml")).is_none());
    }

    #[test]
    fn load_file_valid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "include_dirs = [\"/Users/oxide/src/shared\"]\n").unwrap();
        let raw = load_file(&path).unwrap();
        assert_eq!(
            raw.include_dirs,
            vec![PathBuf::from("/Users/oxide/src/shared")]
        );
    }

    #[test]
    fn load_file_empty_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        let raw = load_file(&path).unwrap();
        assert!(raw.include_dirs.is_empty());
        assert!(raw.engine.is_none());
        assert!(raw.model.is_none());
    }

    #[test]
    fn load_file_engine_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "engine = \"whisper\"\nmodel = \"/custom/model\"\n",
        )
        .unwrap();
        let raw = load_file(&path).unwrap();
        assert_eq!(raw.engine.as_deref(), Some("whisper"));
        assert_eq!(raw.model, Some(PathBuf::from("/custom/model")));
    }

    #[test]
    fn engine_closest_wins() {
        let dir = tempfile::tempdir().unwrap();
        // Parent sets engine = whisper
        let parent_attend = dir.path().join(".attend");
        std::fs::create_dir_all(&parent_attend).unwrap();
        std::fs::write(
            parent_attend.join("config.toml"),
            "engine = \"whisper\"\n",
        )
        .unwrap();

        // Child overrides with engine = parakeet
        let child = dir.path().join("child");
        let child_attend = child.join(".attend");
        std::fs::create_dir_all(&child_attend).unwrap();
        std::fs::write(
            child_attend.join("config.toml"),
            "engine = \"parakeet\"\n",
        )
        .unwrap();

        let config = Config::load(&child);
        // Child (closest) wins
        assert!(matches!(config.engine, Some(Engine::Parakeet)));
    }

    #[test]
    fn hierarchical_walk() {
        let dir = tempfile::tempdir().unwrap();
        // Create parent config
        let parent_attend = dir.path().join(".attend");
        std::fs::create_dir_all(&parent_attend).unwrap();
        std::fs::write(
            parent_attend.join("config.toml"),
            "include_dirs = [\"/parent/lib\"]\n",
        )
        .unwrap();

        // Create child directory with its own config
        let child = dir.path().join("child");
        let child_attend = child.join(".attend");
        std::fs::create_dir_all(&child_attend).unwrap();
        std::fs::write(
            child_attend.join("config.toml"),
            "include_dirs = [\"/child/lib\"]\n",
        )
        .unwrap();

        let config = Config::load(&child);
        // Child config should come first (closer), then parent
        assert!(config.include_dirs.contains(&PathBuf::from("/child/lib")));
        assert!(config.include_dirs.contains(&PathBuf::from("/parent/lib")));
    }
}
