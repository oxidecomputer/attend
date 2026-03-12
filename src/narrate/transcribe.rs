//! Speech-to-text transcription with selectable backend.
//!
//! Supports Whisper (GGML) and Parakeet TDT (ONNX) engines.
//! The engine is chosen via `--engine` on the CLI; Parakeet is the default.

mod parakeet;
pub(crate) mod stub;
mod whisper;

use std::path::Path;

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

    /// Check whether the model files are already cached at `path`.
    pub fn is_model_cached(&self, path: &Utf8Path) -> bool {
        match self {
            Engine::Whisper => path.exists(),
            Engine::Parakeet => parakeet::is_model_cached(path),
        }
    }

    /// Download model files to `path` if not already present (does not load).
    pub fn ensure_model(&self, path: &Utf8Path) -> anyhow::Result<()> {
        match self {
            Engine::Whisper => whisper::ensure_model(path),
            Engine::Parakeet => parakeet::ensure_model(path),
        }
    }

    /// Human-readable engine name for status messages.
    pub fn display_name(&self) -> &'static str {
        match self {
            Engine::Whisper => "Whisper",
            Engine::Parakeet => "Parakeet TDT",
        }
    }

    /// Ensure the model exists (downloading if needed) and load it.
    ///
    /// Logs wall-clock load time at info level for benchmarking.
    pub fn preload(&self, path: &Utf8Path) -> anyhow::Result<Box<dyn Transcriber>> {
        self.ensure_model(path)?;
        let t0 = std::time::Instant::now();
        let transcriber = match self {
            Engine::Whisper => {
                Ok(Box::new(whisper::WhisperTranscriber::load(path)?) as Box<dyn Transcriber>)
            }
            Engine::Parakeet => {
                Ok(Box::new(parakeet::ParakeetTranscriber::load(path)?) as Box<dyn Transcriber>)
            }
        };
        let elapsed = t0.elapsed();
        tracing::info!(
            engine = self.display_name(),
            load_secs = format!("{:.3}", elapsed.as_secs_f64()),
            "Model loaded"
        );
        transcriber
    }

    /// Model variant names for benchmarking.
    pub fn model_names(&self) -> &[&str] {
        match self {
            Engine::Whisper => whisper::MODEL_NAMES,
            Engine::Parakeet => parakeet::MODEL_NAMES,
        }
    }
}

/// Verify a downloaded file against an expected SHA-256 hex digest.
///
/// Returns `Ok(())` if the digest matches, or an error describing the
/// mismatch. Used by engine download functions to detect corrupt or
/// tampered model files.
pub(super) fn verify_sha256(path: &Path, expected_hex: &str) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("cannot open {} for checksum: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_hex {
        anyhow::bail!(
            "checksum mismatch for {}: expected {expected_hex}, got {actual}",
            path.display()
        );
    }
    Ok(())
}
