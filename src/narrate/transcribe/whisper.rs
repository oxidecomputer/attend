//! Whisper (GGML) speech-to-text backend.

use camino::Utf8Path;

use super::{MAX_CHUNK_SAMPLES, SAMPLE_RATE, Word};

/// Model file names available for benchmarking.
pub(super) const MODEL_NAMES: &[&str] = &[
    "ggml-base.en.bin",
    "ggml-small.en.bin",
    "ggml-medium.en.bin",
];

/// Known SHA-256 checksums for well-known Whisper models (from HuggingFace LFS).
/// Models with custom paths or unknown filenames skip verification.
pub(super) fn expected_checksum(filename: &str) -> Option<&'static str> {
    match filename {
        "ggml-base.en.bin" => {
            Some("a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002")
        }
        "ggml-small.en.bin" => {
            Some("c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d")
        }
        "ggml-medium.en.bin" => {
            Some("cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356")
        }
        _ => None,
    }
}

/// Whisper timestamps are in centiseconds (hundredths of a second).
const WHISPER_CENTISEC_DIVISOR: f64 = 100.0;

/// Whisper transcription backend.
pub struct WhisperTranscriber {
    ctx: whisper_rs::WhisperContext,
    /// Persisted state for cross-call context carry-over.
    state: Option<whisper_rs::WhisperState>,
    /// Prior text to seed the next transcription call.
    initial_prompt: Option<String>,
}

impl WhisperTranscriber {
    /// Load a Whisper model from disk.
    pub fn load(model_path: &Utf8Path) -> anyhow::Result<Self> {
        let ctx = whisper_rs::WhisperContext::new_with_params(
            model_path.as_str(),
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|e| anyhow::anyhow!("failed to load whisper model: {e}"))?;
        Ok(Self {
            ctx,
            state: None,
            initial_prompt: None,
        })
    }
}

impl super::Transcriber for WhisperTranscriber {
    fn transcribe(&mut self, samples_16k: &[f32]) -> anyhow::Result<Vec<Word>> {
        use whisper_rs::{FullParams, SamplingStrategy};

        // Lazily create and persist state for cross-call context carry-over.
        if self.state.is_none() {
            self.state = Some(
                self.ctx
                    .create_state()
                    .map_err(|e| anyhow::anyhow!("failed to create whisper state: {e}"))?,
            );
        }
        let state = self.state.as_mut().unwrap();

        let mut words = Vec::new();

        // Split at 4-minute boundaries to stay within Whisper's limits.
        let chunks: Vec<&[f32]> = if samples_16k.len() <= MAX_CHUNK_SAMPLES {
            vec![samples_16k]
        } else {
            samples_16k.chunks(MAX_CHUNK_SAMPLES).collect()
        };

        for (chunk_idx, chunk) in chunks.iter().enumerate() {
            let offset_secs = (chunk_idx * MAX_CHUNK_SAMPLES) as f64 / SAMPLE_RATE as f64;

            // Greedy decoding with best_of=1: fastest strategy, picks the single
            // most likely token at each step. Beam search would improve accuracy
            // on ambiguous segments but costs ~5x more. For real-time narration
            // with on-the-fly transcription, latency matters more than polish.
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

            // Enable per-token timestamps so we can assign each word a time
            // offset within the segment. This is essential for interleaving
            // speech with editor events in chronological order.
            params.set_token_timestamps(true);

            // max_len=1 forces Whisper to emit one segment per token (word).
            // Without this, Whisper batches tokens into multi-word segments,
            // making per-word timestamp resolution impossible.
            params.set_max_len(1);

            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_special(false);
            params.set_print_timestamps(false);

            // English-only: avoids the language detection pass and uses the
            // English-specific vocabulary, improving both speed and accuracy.
            params.set_language(Some("en"));

            // Allow context to carry forward across chunks within the same
            // state. This helps Whisper maintain coherence when audio is split
            // at 4-minute boundaries — the model can reference tokens from the
            // prior chunk to resolve ambiguous words.
            params.set_no_context(false);

            if let Some(ref prompt) = self.initial_prompt {
                params.set_initial_prompt(prompt);
            }

            state
                .full(params, chunk)
                .map_err(|e| anyhow::anyhow!("whisper transcription failed: {e}"))?;

            for segment in state.as_iter() {
                let text = segment
                    .to_str()
                    .map_err(|e| anyhow::anyhow!("failed to get segment text: {e}"))?
                    .trim()
                    .to_string();
                if text.is_empty() {
                    continue;
                }

                // Whisper timestamps are in centiseconds
                words.push(Word {
                    text,
                    start_secs: segment.start_timestamp() as f64 / WHISPER_CENTISEC_DIVISOR
                        + offset_secs,
                    end_secs: segment.end_timestamp() as f64 / WHISPER_CENTISEC_DIVISOR
                        + offset_secs,
                });
            }

            // Clear the initial prompt after the first chunk — subsequent chunks
            // carry context forward via the persisted state.
            self.initial_prompt = None;
        }

        Ok(words)
    }

    fn set_context(&mut self, prior_text: &str) {
        self.initial_prompt = Some(prior_text.to_string());
    }

    fn bench(&mut self, samples: &[f32]) {
        use std::time::Instant;
        use whisper_rs::{FullParams, SamplingStrategy};

        // State creation
        let t1 = Instant::now();
        let mut state = match self.ctx.create_state() {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to create whisper state: {e}");
                return;
            }
        };
        let state_time = t1.elapsed();

        // Transcription
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_token_timestamps(true);
        params.set_max_len(1);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_language(Some("en"));
        params.set_no_context(true);

        let t2 = Instant::now();
        let _ = state.full(params, samples); // Intentionally ignored: bench only measures timing
        let transcribe_time = t2.elapsed();

        let total = state_time + transcribe_time;

        tracing::debug!(
            state_creation_s = state_time.as_secs_f64(),
            transcription_s = transcribe_time.as_secs_f64(),
            total_s = total.as_secs_f64(),
            "Whisper bench"
        );
    }
}

/// Ensure the Whisper model exists at the given path, downloading if needed.
pub(super) fn ensure_model(model_path: &Utf8Path) -> anyhow::Result<()> {
    if model_path.exists() {
        return Ok(());
    }

    let filename = model_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid model path"))?;
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}");

    tracing::info!(path = %model_path, "Downloading Whisper model...");
    super::download_verified(&url, model_path.as_std_path(), expected_checksum(filename), None)?;
    tracing::info!("Whisper model downloaded successfully.");

    Ok(())
}

/// Like [`ensure_model`], but reports download progress via a callback.
pub(super) fn ensure_model_with_progress(
    model_path: &Utf8Path,
    on_progress: &mut dyn FnMut(&str, u64, Option<u64>),
) -> anyhow::Result<()> {
    if model_path.exists() {
        return Ok(());
    }

    let filename = model_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid model path"))?;
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}");

    let mut cb = |bytes, total| on_progress(filename, bytes, total);
    super::download_verified(&url, model_path.as_std_path(), expected_checksum(filename), Some(&mut cb))?;

    Ok(())
}
