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

/// Whisper transcription backend.
pub struct WhisperTranscriber {
    ctx: whisper_rs::WhisperContext,
}

impl WhisperTranscriber {
    /// Load a Whisper model from disk.
    pub fn load(model_path: &Path) -> anyhow::Result<Self> {
        let ctx = whisper_rs::WhisperContext::new_with_params(
            model_path.to_str().unwrap_or_default(),
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|e| anyhow::anyhow!("failed to load whisper model: {e}"))?;
        Ok(Self { ctx })
    }
}

impl super::Transcriber for WhisperTranscriber {
    fn transcribe(&mut self, samples_16k: &[f32]) -> anyhow::Result<Vec<Word>> {
        use whisper_rs::{FullParams, SamplingStrategy};

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| anyhow::anyhow!("failed to create whisper state: {e}"))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_token_timestamps(true);
        params.set_max_len(1);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_language(Some("en"));
        params.set_no_context(true);

        state
            .full(params, samples_16k)
            .map_err(|e| anyhow::anyhow!("whisper transcription failed: {e}"))?;

        let n_segments = state
            .full_n_segments()
            .map_err(|e| anyhow::anyhow!("failed to get segment count: {e}"))?;

        let mut words = Vec::new();

        for i in 0..n_segments {
            let text = state
                .full_get_segment_text(i)
                .map_err(|e| anyhow::anyhow!("failed to get segment text: {e}"))?;
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }

            let start = state
                .full_get_segment_t0(i)
                .map_err(|e| anyhow::anyhow!("failed to get segment t0: {e}"))?;
            let end = state
                .full_get_segment_t1(i)
                .map_err(|e| anyhow::anyhow!("failed to get segment t1: {e}"))?;

            // Whisper timestamps are in centiseconds
            words.push(Word {
                text,
                start_secs: start as f64 / 100.0,
                end_secs: end as f64 / 100.0,
            });
        }

        Ok(words)
    }

    fn bench(&mut self, samples: &[f32]) {
        use std::time::Instant;
        use whisper_rs::{FullParams, SamplingStrategy};

        // State creation
        let t1 = Instant::now();
        let mut state = match self.ctx.create_state() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  Failed to create state: {e}");
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

        eprintln!("  State creation: {:.3}s", state_time.as_secs_f64());
        eprintln!("  Transcription:  {:.3}s", transcribe_time.as_secs_f64());
        eprintln!("  Total:          {:.3}s", total.as_secs_f64());
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
    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{filename}"
    );

    eprintln!("Downloading Whisper model to {}...", model_path.display());

    if let Some(parent) = model_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let response = ureq::get(&url).call()?;
    let mut reader = response.into_body().into_reader();

    let tmp_path = model_path.with_extension("bin.tmp");
    let mut file = fs::File::create(&tmp_path)?;
    std::io::copy(&mut reader, &mut file)?;

    fs::rename(&tmp_path, model_path)?;
    eprintln!("Model downloaded successfully.");

    Ok(())
}
