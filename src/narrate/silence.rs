//! WebRTC VAD-based silence detection.
//!
//! Wraps frame-level voice activity detection into a chunk-level API.
//! When speech is followed by a configurable duration of silence, the
//! detector reports a split point so the caller can transcribe the
//! completed speech segment immediately.

use std::time::{Duration, Instant};

use super::audio::AudioChunk;

/// Internal VAD sample rate — all input is resampled to 16 kHz.
const VAD_RATE: u32 = 16_000;

/// 10 ms frame at 16 kHz.
const FRAME_SAMPLES: usize = (VAD_RATE / 100) as usize;

/// Detects extended silences in an audio stream using WebRTC VAD.
pub struct SilenceDetector {
    vad: webrtc_vad::Vad,
    /// Device sample rate (for resampling to 16 kHz).
    device_sample_rate: u32,
    /// Partial 16 kHz i16 frame carried between chunks.
    leftover: Vec<i16>,
    /// Number of consecutive non-voice 10 ms frames to trigger a split.
    min_silence_frames: usize,
    state: State,
    /// Consecutive non-voice frames in the current trailing silence.
    silent_frames: usize,
    /// Monotonic instant when the current silence began.
    silence_start_instant: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// No speech detected yet (or after a split).
    Idle,
    /// Currently receiving speech.
    Speaking,
    /// Speech ended, counting silence frames.
    Trailing,
}

impl SilenceDetector {
    pub fn new(device_sample_rate: u32, min_silence: Duration) -> Self {
        let vad = webrtc_vad::Vad::new_with_rate_and_mode(
            webrtc_vad::SampleRate::Rate16kHz,
            webrtc_vad::VadMode::Aggressive,
        );
        let min_silence_frames = (min_silence.as_millis() as usize) / 10;

        SilenceDetector {
            vad,
            device_sample_rate,
            leftover: Vec::new(),
            min_silence_frames,
            state: State::Idle,
            silent_frames: 0,
            silence_start_instant: None,
        }
    }

    /// Feed an audio chunk and return `Some(instant)` if a silence-based
    /// split should happen. The returned instant marks the beginning of
    /// the silence — chunks before this instant are speech.
    pub fn feed(&mut self, chunk: &AudioChunk) -> Option<Instant> {
        let resampled = downsample_to_vad(&chunk.samples, self.device_sample_rate);
        self.leftover.extend_from_slice(&resampled);

        let mut result = None;

        while self.leftover.len() >= FRAME_SAMPLES {
            let frame: Vec<i16> = self.leftover.drain(..FRAME_SAMPLES).collect();
            let has_voice = self.vad.is_voice_segment(&frame).unwrap_or(false);

            match self.state {
                State::Idle => {
                    if has_voice {
                        tracing::debug!("VAD: voice detected — Idle → Speaking");
                        self.state = State::Speaking;
                        self.silent_frames = 0;
                    }
                }
                State::Speaking => {
                    if !has_voice {
                        tracing::debug!("VAD: silence detected — Speaking → Trailing");
                        self.state = State::Trailing;
                        self.silent_frames = 1;
                        self.silence_start_instant = Some(chunk.instant);
                    }
                }
                State::Trailing => {
                    if has_voice {
                        tracing::debug!(
                            after_frames = self.silent_frames,
                            "VAD: voice resumed — Trailing → Speaking"
                        );
                        self.state = State::Speaking;
                        self.silent_frames = 0;
                        self.silence_start_instant = None;
                    } else {
                        self.silent_frames += 1;
                        // Log progress every second of silence.
                        if self.silent_frames.is_multiple_of(100) {
                            tracing::debug!(
                                silent_secs = self.silent_frames as f64 / 100.0,
                                threshold_secs = self.min_silence_frames as f64 / 100.0,
                                "VAD: silence continuing"
                            );
                        }
                        if self.silent_frames >= self.min_silence_frames {
                            tracing::info!(
                                silent_secs = self.silent_frames as f64 / 100.0,
                                "VAD: silence threshold reached — splitting segment"
                            );
                            result = self.silence_start_instant.take();
                            self.state = State::Idle;
                            self.silent_frames = 0;
                        }
                    }
                }
            }
        }

        result
    }

    /// Reset to idle state, clearing any partial frame and counters.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.silent_frames = 0;
        self.silence_start_instant = None;
        self.leftover.clear();
    }
}

/// Cheap linear-interpolation resample from any device rate to 16 kHz i16.
///
/// Quality is adequate for VAD (which internally downsamples to 8 kHz anyway).
fn downsample_to_vad(samples: &[f32], from_rate: u32) -> Vec<i16> {
    if from_rate == VAD_RATE {
        return samples
            .iter()
            .map(|&s| f32_to_i16(s))
            .collect();
    }

    let ratio = from_rate as f64 / VAD_RATE as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = src_pos - idx as f64;
        let s = if idx + 1 < samples.len() {
            samples[idx] as f64 * (1.0 - frac) + samples[idx + 1] as f64 * frac
        } else {
            samples[idx.min(samples.len().saturating_sub(1))] as f64
        };
        out.push(f32_to_i16(s as f32));
    }

    out
}

fn f32_to_i16(s: f32) -> i16 {
    (s * 32767.0).clamp(-32768.0, 32767.0) as i16
}

#[cfg(test)]
mod tests {
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
}
