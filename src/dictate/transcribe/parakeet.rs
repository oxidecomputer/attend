//! Parakeet CTC (ONNX) speech-to-text backend.

use std::fs;
use std::path::Path;

use parakeet_rs::Transcriber as _;
use parakeet_rs::TimestampMode;

use super::Word;

/// Model variant names for benchmarking.
pub(super) const MODEL_NAMES: &[&str] = &["parakeet-ctc-0.6b"];

/// Maximum chunk length in samples (4 minutes at 16 kHz).
const MAX_CHUNK_SAMPLES: usize = 240 * 16_000;

const REPO: &str = "onnx-community/parakeet-ctc-0.6b-ONNX";

/// (local filename, repo path) pairs for required model files.
const MODEL_FILES: &[(&str, &str)] = &[
    ("model.onnx", "onnx/model.onnx"),
    ("model.onnx_data", "onnx/model.onnx_data"),
    ("tokenizer.json", "tokenizer.json"),
];

/// Parakeet CTC transcription backend.
pub struct ParakeetTranscriber {
    model: parakeet_rs::Parakeet,
}

impl ParakeetTranscriber {
    /// Load a Parakeet model from a directory.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let model = parakeet_rs::Parakeet::from_pretrained(
            dir.to_str().unwrap_or_default(),
            None,
        )?;
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
        let _ = self.model.transcribe_samples(samples.to_vec(), 16_000, 1, Some(TimestampMode::Words));
        let transcribe_time = t0.elapsed();

        eprintln!("  Transcription:  {:.3}s", transcribe_time.as_secs_f64());
    }
}

/// Ensure the Parakeet model directory exists with all required files.
pub(super) fn ensure_model(dir: &Path) -> anyhow::Result<()> {
    let all_present = MODEL_FILES.iter().all(|(local, _)| dir.join(local).exists());
    if all_present {
        return Ok(());
    }
    download_model(dir)
}

fn download_model(dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;

    for &(local, repo_path) in MODEL_FILES {
        let dest = dir.join(local);
        if dest.exists() {
            continue;
        }

        let url = format!(
            "https://huggingface.co/{REPO}/resolve/main/{repo_path}"
        );
        eprintln!("Downloading {local} to {}...", dir.display());

        let response = ureq::get(&url).call()?;
        let mut reader = response.into_body().into_reader();

        let tmp_path = dest.with_extension("tmp");
        let mut file = fs::File::create(&tmp_path)?;
        std::io::copy(&mut reader, &mut file)?;

        fs::rename(&tmp_path, &dest)?;
    }

    eprintln!("Parakeet model downloaded successfully.");
    Ok(())
}
