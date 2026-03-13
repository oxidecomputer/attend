//! Parakeet TDT (ONNX) speech-to-text backend.
//!
//! Uses the TDT (Token-and-Duration Transducer) variant which predicts
//! punctuation and capitalization. Word-level timestamps enable accurate
//! interleaving with external selection events. The TDT decoder correctly
//! accounts for 8x encoder subsampling in its timestamps, unlike the CTC
//! decoder.

use std::fs;

use camino::Utf8Path;

use parakeet_rs::TimestampMode;
use parakeet_rs::Transcriber as _;

use super::{MAX_CHUNK_SAMPLES, SAMPLE_RATE, Word};

/// Model variant names for benchmarking.
pub(super) const MODEL_NAMES: &[&str] = &["parakeet-tdt-0.6b-v3"];

const REPO: &str = "istupakov/parakeet-tdt-0.6b-v3-onnx";

/// Required model files (all at repo root).
pub(super) const MODEL_FILES: &[&str] = &[
    "encoder-model.onnx",
    "encoder-model.onnx.data",
    "decoder_joint-model.onnx",
    "vocab.txt",
];

/// Known SHA-256 checksums for well-known Parakeet model files (from HuggingFace LFS).
/// Only LFS-tracked files have known checksums; small files (vocab.txt) skip verification.
pub(super) fn expected_checksum(filename: &str) -> Option<&'static str> {
    match filename {
        "encoder-model.onnx" => {
            Some("98a74b21b4cc0017c1e7030319a4a96f4a9506e50f0708f3a516d02a77c96bb1")
        }
        "encoder-model.onnx.data" => {
            Some("9a22d372c51455c34f13405da2520baefb7125bd16981397561423ed32d24f36")
        }
        "decoder_joint-model.onnx" => {
            Some("e978ddf6688527182c10fde2eb4b83068421648985ef23f7a86be732be8706c1")
        }
        _ => None,
    }
}

/// Parakeet TDT transcription backend.
pub struct ParakeetTranscriber {
    model: parakeet_rs::ParakeetTDT,
}

impl ParakeetTranscriber {
    /// Load a Parakeet TDT model from a directory.
    pub fn load(dir: &Utf8Path) -> anyhow::Result<Self> {
        let model = parakeet_rs::ParakeetTDT::from_pretrained(dir.as_str(), None)?;
        Ok(Self { model })
    }
}

impl super::Transcriber for ParakeetTranscriber {
    fn transcribe(&mut self, samples_16k: &[f32]) -> anyhow::Result<Vec<Word>> {
        let mut words = Vec::new();

        // Chunk long audio at 4-minute boundaries to stay within model limits.
        let chunks: Vec<&[f32]> = if samples_16k.len() <= MAX_CHUNK_SAMPLES {
            vec![samples_16k]
        } else {
            samples_16k.chunks(MAX_CHUNK_SAMPLES).collect()
        };

        for (chunk_idx, chunk) in chunks.iter().enumerate() {
            let offset_secs = (chunk_idx * MAX_CHUNK_SAMPLES) as f64 / SAMPLE_RATE as f64;

            let result = self.model.transcribe_samples(
                chunk.to_vec(),
                SAMPLE_RATE,
                1,
                Some(TimestampMode::Words),
            )?;

            for token in result.tokens {
                let text = token.text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                words.push(Word {
                    text,
                    start_secs: token.start as f64 + offset_secs,
                    end_secs: token.end as f64 + offset_secs,
                });
            }
        }

        Ok(words)
    }

    fn bench(&mut self, samples: &[f32]) {
        use std::time::Instant;

        let t0 = Instant::now();
        // Intentionally ignored: bench only measures timing
        let _ = self.model.transcribe_samples(
            samples.to_vec(),
            SAMPLE_RATE,
            1,
            Some(TimestampMode::Words),
        );
        let transcribe_time = t0.elapsed();

        tracing::debug!(
            transcription_s = transcribe_time.as_secs_f64(),
            "Parakeet bench"
        );
    }
}

/// Check whether all Parakeet model files are present.
pub(super) fn is_model_cached(dir: &Utf8Path) -> bool {
    MODEL_FILES.iter().all(|f| dir.join(f).exists())
}

/// Ensure the Parakeet TDT model directory exists with all required files.
pub(super) fn ensure_model(dir: &Utf8Path) -> anyhow::Result<()> {
    if is_model_cached(dir) {
        return Ok(());
    }

    fs::create_dir_all(dir)?;

    for filename in MODEL_FILES {
        let dest = dir.join(filename);
        if dest.exists() {
            continue;
        }

        let url = format!("https://huggingface.co/{REPO}/resolve/main/{filename}");
        tracing::info!(filename, dir = %dir, "Downloading Parakeet model file...");
        super::download_verified(&url, dest.as_std_path(), expected_checksum(filename))?;
    }

    tracing::info!("Parakeet TDT model downloaded successfully.");
    Ok(())
}
