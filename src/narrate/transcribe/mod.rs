//! Speech-to-text transcription with selectable backend.
//!
//! Supports Whisper (GGML) and Parakeet TDT (ONNX) engines.
//! The engine is chosen via `--engine` on the CLI; Parakeet is the default.

mod parakeet;
mod whisper;

use camino::{Utf8Path, Utf8PathBuf};

/// A single transcribed word with its timing.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Word {
    /// The word text.
    pub text: String,
    /// Start time in seconds relative to audio start.
    pub start_secs: f64,
    /// End time in seconds relative to audio start.
    pub end_secs: f64,
}

/// Trait implemented by each speech-to-text backend.
pub trait Transcriber: Send {
    /// Transcribe 16 kHz mono f32 samples to words with timestamps.
    fn transcribe(&mut self, samples_16k: &[f32]) -> anyhow::Result<Vec<Word>>;

    /// Provide prior transcription text as context for the next `transcribe()` call.
    ///
    /// Backends that support prompting (Whisper) use this to improve consistency
    /// across segment boundaries. Backends without prompt support (Parakeet) ignore it.
    fn set_context(&mut self, _prior_text: &str) {}

    /// Run benchmarks for this engine. Prints results to stderr.
    fn bench(&mut self, samples: &[f32]);
}

/// CLI-selectable transcription engine.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Whisper,
    Parakeet,
}

impl Engine {
    /// Default model path for this engine.
    pub fn default_model_path(&self) -> Utf8PathBuf {
        let models = super::cache_dir().join("models");
        match self {
            Engine::Whisper => models.join("ggml-small.en.bin"),
            Engine::Parakeet => models.join("parakeet-tdt-0.6b-v3"),
        }
    }

    /// Ensure the model exists (downloading if needed) and load it.
    pub fn preload(&self, path: &Utf8Path) -> anyhow::Result<Box<dyn Transcriber>> {
        match self {
            Engine::Whisper => {
                whisper::ensure_model(path)?;
                Ok(Box::new(whisper::WhisperTranscriber::load(path)?))
            }
            Engine::Parakeet => {
                parakeet::ensure_model(path)?;
                Ok(Box::new(parakeet::ParakeetTranscriber::load(path)?))
            }
        }
    }

    /// Model variant names for benchmarking.
    pub fn model_names(&self) -> &[&str] {
        match self {
            Engine::Whisper => whisper::MODEL_NAMES,
            Engine::Parakeet => parakeet::MODEL_NAMES,
        }
    }

    /// Ensure a model variant and load it (for benchmarking).
    pub fn ensure_and_load(&self, path: &Utf8Path) -> anyhow::Result<Box<dyn Transcriber>> {
        self.preload(path)
    }
}
