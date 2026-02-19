//! Whisper (GGML) speech-to-text backend.

use std::fs;
use std::path::Path;

use super::Word;

/// Model file names available for benchmarking.
pub(super) const MODEL_NAMES: &[&str] = &[
    "ggml-base.en.bin",
    "ggml-small.en.bin",
    "ggml-medium.en.bin",
];

/// Maximum chunk length in samples (4 minutes at 16 kHz).
const MAX_CHUNK_SAMPLES: usize = 240 * 16_000;

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
    pub fn load(model_path: &Path) -> anyhow::Result<Self> {
        let ctx = whisper_rs::WhisperContext::new_with_params(
            model_path.to_str().unwrap_or_default(),
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
            let offset_secs = (chunk_idx * MAX_CHUNK_SAMPLES) as f64 / 16_000.0;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_token_timestamps(true);
            params.set_max_len(1);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_special(false);
            params.set_print_timestamps(false);
            params.set_language(Some("en"));
            // Allow context to carry forward across chunks within the same state.
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
                    start_secs: segment.start_timestamp() as f64 / 100.0 + offset_secs,
                    end_secs: segment.end_timestamp() as f64 / 100.0 + offset_secs,
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
        let _ = state.full(params, samples);
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
pub(super) fn ensure_model(model_path: &Path) -> anyhow::Result<()> {
    if model_path.exists() {
        return Ok(());
    }
    download_model(model_path)
}

fn download_model(model_path: &Path) -> anyhow::Result<()> {
    let filename = model_path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid model path"))?;
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}");

    tracing::info!(path = %model_path.display(), "Downloading Whisper model...");

    if let Some(parent) = model_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let response = ureq::get(&url).call()?;
    let mut reader = response.into_body().into_reader();

    let tmp_path = model_path.with_extension("bin.tmp");
    let mut file = fs::File::create(&tmp_path)?;
    std::io::copy(&mut reader, &mut file)?;

    fs::rename(&tmp_path, model_path)?;
    tracing::info!("Whisper model downloaded successfully.");

    Ok(())
}
