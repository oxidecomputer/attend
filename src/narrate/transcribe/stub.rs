//! Stub transcriber for end-to-end testing.
//!
//! Returns injected text as synthetic [`Word`]s without loading any model
//! or processing audio. The test harness sends `(text, duration_ms)` pairs
//! via the channel returned by [`StubTranscriber::new`]; each subsequent
//! `transcribe()` call pops the next pair and generates evenly-spaced words.
//!
//! Used when `ATTEND_TEST_MODE=1` to eliminate model download, GPU/CPU
//! transcription, and real audio dependencies from the test loop.

use std::sync::mpsc;

use super::{Transcriber, Word};

/// A speech injection: the text that was "said" and how long it took.
pub struct Injection {
    pub text: String,
    pub duration_ms: u64,
}

/// Test-only transcriber that returns pre-injected text.
#[derive(Debug)]
pub struct StubTranscriber {
    rx: mpsc::Receiver<Injection>,
}

impl StubTranscriber {
    /// Create a new stub transcriber and its injection channel.
    ///
    /// The caller holds the `Sender` and pushes [`Injection`]s; each
    /// `transcribe()` call drains all pending injections and returns
    /// synthetic words.
    pub fn new() -> (Self, mpsc::Sender<Injection>) {
        let (tx, rx) = mpsc::channel();
        (Self { rx }, tx)
    }
}

impl Transcriber for StubTranscriber {
    /// Drain all pending injections and return synthetic words.
    ///
    /// Each injection's text is split on whitespace. Words are evenly
    /// spaced across the injection's `duration_ms`. If no injections
    /// are pending, returns an empty vec (as if silence was transcribed).
    fn transcribe(&mut self, _samples_16k: &[f32]) -> anyhow::Result<Vec<Word>> {
        let mut words = Vec::new();
        let mut time_offset_secs = 0.0;

        // Drain all available injections without blocking.
        while let Ok(injection) = self.rx.try_recv() {
            let duration_secs = injection.duration_ms as f64 / 1000.0;
            let tokens: Vec<&str> = injection.text.split_whitespace().collect();

            if tokens.is_empty() {
                time_offset_secs += duration_secs;
                continue;
            }

            let word_duration = duration_secs / tokens.len() as f64;
            for (i, token) in tokens.iter().enumerate() {
                let start = time_offset_secs + i as f64 * word_duration;
                words.push(Word {
                    text: token.to_string(),
                    start_secs: start,
                    end_secs: start + word_duration,
                });
            }
            time_offset_secs += duration_secs;
        }

        Ok(words)
    }

    /// No-op: stub doesn't use context for consistency.
    fn set_context(&mut self, _prior_text: &str) {}

    /// No-op: stub has nothing to benchmark.
    fn bench(&mut self, _samples: &[f32]) {}
}

#[cfg(test)]
mod tests;
