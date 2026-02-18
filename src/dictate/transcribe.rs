//! Whisper-based speech transcription with word-level timestamps.
//!
//! Loads a GGML Whisper model (auto-downloading on first use) and
//! transcribes audio to text with per-word timing information.

#[cfg(feature = "dictate")]
use std::fs;
use std::path::Path;

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

/// Ensure the Whisper model exists at the given path, downloading if needed.
pub fn ensure_model(model_path: &Path) -> anyhow::Result<()> {
    if model_path.exists() {
        return Ok(());
    }

    #[cfg(feature = "dictate")]
    {
        download_model(model_path)
    }
    #[cfg(not(feature = "dictate"))]
    {
        anyhow::bail!(
            "model not found at {} (download requires `dictate` feature)",
            model_path.display()
        )
    }
}

#[cfg(feature = "dictate")]
fn download_model(model_path: &Path) -> anyhow::Result<()> {
    let url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

    eprintln!("Downloading Whisper model to {}...", model_path.display());

    if let Some(parent) = model_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let response = ureq::get(url).call()?;
    let mut reader = response.into_body().into_reader();

    let tmp_path = model_path.with_extension("bin.tmp");
    let mut file = fs::File::create(&tmp_path)?;
    std::io::copy(&mut reader, &mut file)?;

    fs::rename(&tmp_path, model_path)?;
    eprintln!("Model downloaded successfully.");

    Ok(())
}

/// Transcribe 16 kHz mono f32 audio to words with timestamps.
pub fn transcribe(samples_16k: &[f32], model_path: &Path) -> anyhow::Result<Vec<Word>> {
    ensure_model(model_path)?;

    #[cfg(feature = "dictate")]
    {
        transcribe_impl(samples_16k, model_path)
    }
    #[cfg(not(feature = "dictate"))]
    {
        anyhow::bail!("transcription requires the `dictate` feature")
    }
}

#[cfg(feature = "dictate")]
fn transcribe_impl(samples_16k: &[f32], model_path: &Path) -> anyhow::Result<Vec<Word>> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let ctx = WhisperContext::new_with_params(
        model_path.to_str().unwrap_or_default(),
        WhisperContextParameters::default(),
    )
    .map_err(|e| anyhow::anyhow!("failed to load whisper model: {e}"))?;

    let mut state = ctx
        .create_state()
        .map_err(|e| anyhow::anyhow!("failed to create whisper state: {e}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_token_timestamps(true);
    params.set_max_len(1); // one token per segment for word-level timing
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
