use super::*;

fn make_chunk(samples: Vec<f32>, instant: Instant) -> AudioChunk {
    AudioChunk {
        wall_clock: String::new(),
        instant,
        samples,
    }
}

#[test]
fn idle_with_silence() {
    let mut det = SilenceDetector::new(16_000, Duration::from_secs(1));
    let now = Instant::now();
    // Feed silence — should stay idle, no split.
    let chunk = make_chunk(vec![0.0; 16_000], now);
    assert!(det.feed(&chunk).is_none());
    assert_eq!(det.state, State::Idle);
}

#[test]
fn downsample_preserves_length() {
    // 480 samples at 48 kHz = 10ms → should produce ~160 samples at 16 kHz
    let samples = vec![0.1f32; 480];
    let out = downsample_to_vad(&samples, 48_000);
    assert_eq!(out.len(), 160);
}

#[test]
fn downsample_identity_at_16k() {
    let samples = vec![0.5f32; 160];
    let out = downsample_to_vad(&samples, 16_000);
    assert_eq!(out.len(), 160);
    // Should be close to 0.5 * 32767
    assert!((out[0] as f64 - 16383.5).abs() < 1.0);
}

#[test]
fn reset_clears_state() {
    let mut det = SilenceDetector::new(16_000, Duration::from_secs(1));
    det.state = State::Speaking;
    det.silent_frames = 42;
    det.leftover = vec![1, 2, 3];
    det.silence_start_instant = Some(Instant::now());

    det.reset();

    assert_eq!(det.state, State::Idle);
    assert_eq!(det.silent_frames, 0);
    assert!(det.leftover.is_empty());
    assert!(det.silence_start_instant.is_none());
}
