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
mod tests {
    use super::*;

    /// Injected words are returned with evenly-spaced timestamps.
    #[test]
    fn synthetic_timestamps_are_evenly_spaced() {
        let (mut stub, tx) = StubTranscriber::new();

        tx.send(Injection {
            text: "hello world".into(),
            duration_ms: 2000,
        })
        .unwrap();

        let words = stub.transcribe(&[]).unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "hello");
        assert_eq!(words[1].text, "world");

        // 2s / 2 words = 1s per word
        assert!((words[0].start_secs - 0.0).abs() < 1e-9);
        assert!((words[0].end_secs - 1.0).abs() < 1e-9);
        assert!((words[1].start_secs - 1.0).abs() < 1e-9);
        assert!((words[1].end_secs - 2.0).abs() < 1e-9);
    }

    /// Multiple injections are drained in a single transcribe() call
    /// with cumulative time offsets.
    #[test]
    fn multiple_injections_accumulate_time() {
        let (mut stub, tx) = StubTranscriber::new();

        tx.send(Injection {
            text: "first".into(),
            duration_ms: 1000,
        })
        .unwrap();
        tx.send(Injection {
            text: "second".into(),
            duration_ms: 1000,
        })
        .unwrap();

        let words = stub.transcribe(&[]).unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "first");
        assert_eq!(words[1].text, "second");

        // first: 0-1s, second: 1-2s
        assert!((words[0].start_secs - 0.0).abs() < 1e-9);
        assert!((words[1].start_secs - 1.0).abs() < 1e-9);
    }

    /// No pending injections returns empty (silence).
    #[test]
    fn no_injections_returns_empty() {
        let (mut stub, _tx) = StubTranscriber::new();
        let words = stub.transcribe(&[]).unwrap();
        assert!(words.is_empty());
    }

    /// Whitespace-only injection text produces no words but advances time.
    #[test]
    fn whitespace_only_injection_advances_time() {
        let (mut stub, tx) = StubTranscriber::new();

        tx.send(Injection {
            text: "   ".into(),
            duration_ms: 1000,
        })
        .unwrap();
        tx.send(Injection {
            text: "after".into(),
            duration_ms: 1000,
        })
        .unwrap();

        let words = stub.transcribe(&[]).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "after");
        // Whitespace injection consumed 1s, so "after" starts at 1s.
        assert!((words[0].start_secs - 1.0).abs() < 1e-9);
    }

    /// Dropped sender doesn't cause errors — just returns whatever
    /// was buffered.
    #[test]
    fn dropped_sender_is_graceful() {
        let (mut stub, tx) = StubTranscriber::new();

        tx.send(Injection {
            text: "buffered".into(),
            duration_ms: 500,
        })
        .unwrap();
        drop(tx);

        let words = stub.transcribe(&[]).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "buffered");

        // Second call after sender dropped: empty, no error.
        let words = stub.transcribe(&[]).unwrap();
        assert!(words.is_empty());
    }
}
