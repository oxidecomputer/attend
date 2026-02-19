//! Parakeet TDT (ONNX) speech-to-text backend.
//!
//! Uses the TDT (Token-and-Duration Transducer) variant which predicts
//! punctuation and capitalization, enabling natural sentence boundaries.
//! The TDT decoder also correctly accounts for 8x encoder subsampling
//! in its timestamps, unlike the CTC decoder.

use std::fs;
use std::path::Path;

use parakeet_rs::TimestampMode;
use parakeet_rs::Transcriber as _;

use super::Word;

/// Model variant names for benchmarking.
pub(super) const MODEL_NAMES: &[&str] = &["parakeet-tdt-0.6b-v3"];

/// Maximum chunk length in samples (4 minutes at 16 kHz).
const MAX_CHUNK_SAMPLES: usize = 240 * 16_000;

const REPO: &str = "istupakov/parakeet-tdt-0.6b-v3-onnx";

/// Required model files (all at repo root).
const MODEL_FILES: &[&str] = &[
    "encoder-model.onnx",
    "encoder-model.onnx.data",
    "decoder_joint-model.onnx",
    "vocab.txt",
];

/// Parakeet TDT transcription backend.
pub struct ParakeetTranscriber {
    model: parakeet_rs::ParakeetTDT,
}

impl ParakeetTranscriber {
    /// Load a Parakeet TDT model from a directory.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let model =
            parakeet_rs::ParakeetTDT::from_pretrained(dir.to_str().unwrap_or_default(), None)?;
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
            let offset_secs = (chunk_idx * MAX_CHUNK_SAMPLES) as f64 / 16_000.0;

            let result = self.model.transcribe_samples(
                chunk.to_vec(),
                16_000,
                1,
                Some(TimestampMode::Sentences),
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
        let _ = self.model.transcribe_samples(
            samples.to_vec(),
            16_000,
            1,
            Some(TimestampMode::Sentences),
        );
        let transcribe_time = t0.elapsed();

        tracing::debug!(
            transcription_s = transcribe_time.as_secs_f64(),
            "Parakeet bench"
        );
    }
}

/// Ensure the Parakeet TDT model directory exists with all required files.
pub(super) fn ensure_model(dir: &Path) -> anyhow::Result<()> {
    let all_present = MODEL_FILES.iter().all(|f| dir.join(f).exists());
    if all_present {
        return Ok(());
    }
    download_model(dir)
}

fn download_model(dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;

    for filename in MODEL_FILES {
        let dest = dir.join(filename);
        if dest.exists() {
            continue;
        }

        let url = format!("https://huggingface.co/{REPO}/resolve/main/{filename}");
        tracing::info!(filename, dir = %dir.display(), "Downloading Parakeet model file...");

        let response = ureq::get(&url).call()?;
        let mut reader = response.into_body().into_reader();

        let tmp_path = dest.with_extension("tmp");
        let mut file = fs::File::create(&tmp_path)?;
        std::io::copy(&mut reader, &mut file)?;

        fs::rename(&tmp_path, &dest)?;
    }

    tracing::info!("Parakeet TDT model downloaded successfully.");
    Ok(())
}
