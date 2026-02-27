use chrono::{Duration, Utc};

use super::*;

fn make_chunk(samples: Vec<f32>, timestamp: DateTime<Utc>) -> AudioChunk {
    AudioChunk { timestamp, samples }
}

/// Generate a speech-like signal at 16 kHz: sum of harmonics (fundamental +
/// overtones) typical of human voice, with enough spectral energy to trigger
/// WebRTC VAD in Aggressive mode.
fn speech_signal(n_samples: usize) -> Vec<f32> {
    // Fundamental ≈ 150 Hz with harmonics up to ~3400 Hz (telephone band).
    let freqs = [150.0, 300.0, 450.0, 600.0, 900.0, 1200.0, 2000.0, 3400.0];
    let rate = VAD_RATE as f32;
    (0..n_samples)
        .map(|i| {
            let t = i as f32 / rate;
            let sum: f32 = freqs
                .iter()
                .enumerate()
                .map(|(k, &f)| {
                    // Decay higher harmonics (1/k roll-off like voice).
                    let amp = 1.0 / (k as f32 + 1.0);
                    amp * (2.0 * std::f32::consts::PI * f * t).sin()
                })
                .sum();
            // Normalize to ±0.7 (high enough for VAD, below clipping).
            sum * 0.3
        })
        .collect()
}

/// Generate pure silence at 16 kHz.
fn silence_signal(n_samples: usize) -> Vec<f32> {
    vec![0.0; n_samples]
}

/// Feeding pure silence keeps the detector idle with no split point.
#[test]
fn idle_with_silence() {
    let mut det = SilenceDetector::new(16_000, std::time::Duration::from_secs(1));
    let now = Utc::now();
    // Feed silence — should stay idle, no split.
    let chunk = make_chunk(vec![0.0; 16_000], now);
    assert!(det.feed(&chunk).is_none());
    assert_eq!(det.state, State::Idle);
}

/// Downsampling 480 samples at 48 kHz to 16 kHz produces 160 samples.
#[test]
fn downsample_preserves_length() {
    // 480 samples at 48 kHz = 10ms → should produce ~160 samples at 16 kHz
    let samples = vec![0.1f32; 480];
    let out = downsample_to_vad(&samples, 48_000);
    assert_eq!(out.len(), 160);
}

/// Downsampling at the target rate (16 kHz) preserves sample count and scales to i16.
#[test]
fn downsample_identity_at_16k() {
    let samples = vec![0.5f32; 160];
    let out = downsample_to_vad(&samples, 16_000);
    assert_eq!(out.len(), 160);
    // Should be close to 0.5 * 32767
    assert!((out[0] as f64 - 16383.5).abs() < 1.0);
}

/// Resetting the detector clears all accumulated state back to idle.
#[test]
fn reset_clears_state() {
    let mut det = SilenceDetector::new(16_000, std::time::Duration::from_secs(1));
    det.state = State::Speaking;
    det.silent_frames = 42;
    det.leftover = vec![1, 2, 3];
    det.silence_start = Some(Utc::now());

    det.reset();

    assert_eq!(det.state, State::Idle);
    assert_eq!(det.silent_frames, 0);
    assert!(det.leftover.is_empty());
    assert!(det.silence_start.is_none());
}

// ── Integration tests: synthesized audio ────────────────────────────────────

/// Speech followed by silence exceeding the threshold triggers a split.
/// The returned timestamp marks the beginning of the silence.
#[test]
fn speech_then_silence_triggers_split() {
    let silence_dur = std::time::Duration::from_millis(500);
    let mut det = SilenceDetector::new(VAD_RATE, silence_dur);
    let now = Utc::now();

    // 500ms of speech-like signal (8000 samples at 16 kHz).
    let speech = speech_signal(8000);
    let speech_chunk = make_chunk(speech, now);
    assert!(det.feed(&speech_chunk).is_none(), "no split during speech");
    assert_eq!(det.state, State::Speaking, "should be speaking after voice");

    // 700ms of silence (11200 samples) — exceeds 500ms threshold.
    let silence_ts = now + Duration::milliseconds(500);
    let silence = silence_signal(11200);
    let silence_chunk = make_chunk(silence, silence_ts);
    let split = det.feed(&silence_chunk);

    assert!(split.is_some(), "should split after extended silence");
    assert_eq!(
        split.unwrap(),
        silence_ts,
        "split timestamp should mark silence start"
    );
    assert_eq!(det.state, State::Idle, "should return to idle after split");
}

/// Continuous speech signal produces no split points.
#[test]
fn continuous_speech_no_split() {
    let mut det = SilenceDetector::new(VAD_RATE, std::time::Duration::from_millis(500));
    let now = Utc::now();

    // Feed 2 seconds of speech in 100ms chunks.
    for i in 0..20 {
        let chunk_ts = now + Duration::milliseconds(i * 100);
        let speech = speech_signal(1600); // 100ms at 16 kHz
        let chunk = make_chunk(speech, chunk_ts);
        assert!(
            det.feed(&chunk).is_none(),
            "split should not fire during continuous speech (chunk {i})"
        );
    }
    // Should still be speaking.
    assert_eq!(det.state, State::Speaking);
}

/// A short silence gap (below threshold) followed by resumed speech does not split.
#[test]
fn short_silence_no_split() {
    let silence_dur = std::time::Duration::from_millis(500);
    let mut det = SilenceDetector::new(VAD_RATE, silence_dur);
    let now = Utc::now();

    // 300ms of speech.
    let speech1 = speech_signal(4800);
    let chunk1 = make_chunk(speech1, now);
    assert!(det.feed(&chunk1).is_none());

    // 300ms of silence (below 500ms threshold).
    let t1 = now + Duration::milliseconds(300);
    let short_silence = silence_signal(4800);
    let chunk2 = make_chunk(short_silence, t1);
    assert!(
        det.feed(&chunk2).is_none(),
        "short silence should not trigger split"
    );

    // Resume speech — VAD should return to Speaking.
    let t2 = now + Duration::milliseconds(600);
    let speech2 = speech_signal(4800);
    let chunk3 = make_chunk(speech2, t2);
    assert!(det.feed(&chunk3).is_none());
    assert_eq!(
        det.state,
        State::Speaking,
        "should resume speaking after short gap"
    );
}

/// Multiple speech/silence cycles produce one split per silence that exceeds
/// the threshold.
#[test]
fn multiple_cycles_produce_multiple_splits() {
    let silence_dur = std::time::Duration::from_millis(500);
    let mut det = SilenceDetector::new(VAD_RATE, silence_dur);
    let now = Utc::now();
    let mut splits = Vec::new();

    // Cycle 1: 500ms speech + 700ms silence
    let speech1 = speech_signal(8000);
    det.feed(&make_chunk(speech1, now));
    let t1 = now + Duration::milliseconds(500);
    let silence1 = silence_signal(11200);
    if let Some(ts) = det.feed(&make_chunk(silence1, t1)) {
        splits.push(ts);
    }

    // Cycle 2: 500ms speech + 700ms silence
    // After split, detector is Idle — needs enough speech to re-trigger.
    let t2 = now + Duration::milliseconds(1200);
    let speech2 = speech_signal(8000);
    det.feed(&make_chunk(speech2, t2));
    let t3 = now + Duration::milliseconds(1700);
    let silence2 = silence_signal(11200);
    if let Some(ts) = det.feed(&make_chunk(silence2, t3)) {
        splits.push(ts);
    }

    assert_eq!(splits.len(), 2, "should produce two split points");
    assert_eq!(splits[0], t1, "first split at start of first silence");
    assert_eq!(splits[1], t3, "second split at start of second silence");
}

/// Silence before any speech does not produce a split (detector stays Idle).
#[test]
fn initial_silence_no_split() {
    let mut det = SilenceDetector::new(VAD_RATE, std::time::Duration::from_millis(500));
    let now = Utc::now();

    // 2 seconds of silence with no preceding speech.
    let silence = silence_signal(32_000);
    let chunk = make_chunk(silence, now);
    assert!(
        det.feed(&chunk).is_none(),
        "silence without speech should not split"
    );
    assert_eq!(det.state, State::Idle);
}

/// The detector works correctly with 48 kHz input (downsampling path).
#[test]
fn works_at_48khz() {
    let silence_dur = std::time::Duration::from_millis(500);
    let mut det = SilenceDetector::new(48_000, silence_dur);
    let now = Utc::now();

    // 500ms of speech at 48 kHz (24000 samples).
    // Generate at 16 kHz then upsample naively (repeat each sample 3x).
    let speech_16k = speech_signal(8000);
    let speech_48k: Vec<f32> = speech_16k.iter().flat_map(|&s| [s, s, s]).collect();
    let chunk1 = make_chunk(speech_48k, now);
    assert!(det.feed(&chunk1).is_none());

    // 700ms of silence at 48 kHz (33600 samples).
    let t1 = now + Duration::milliseconds(500);
    let silence_48k = silence_signal(33600);
    let chunk2 = make_chunk(silence_48k, t1);
    let split = det.feed(&chunk2);

    assert!(split.is_some(), "should split with 48 kHz input");
    assert_eq!(det.state, State::Idle);
}
