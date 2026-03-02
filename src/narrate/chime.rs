//! Auditory feedback chimes for narration state transitions.
//!
//! Each [`Chime`] variant corresponds to a user-facing event (start recording,
//! stop, pause, etc.). All chimes use the same amplitude and are rendered as
//! sine-wave notes with a smooth envelope to avoid clicks.

use std::sync::Arc;

// ---------------------------------------------------------------------------
// Note frequencies (Hz)
// ---------------------------------------------------------------------------

const NOTE_A4: f32 = 440.0;
const NOTE_C5: f32 = 523.25;
const NOTE_D5: f32 = 587.33;
const NOTE_E5: f32 = 659.25;
const NOTE_G5: f32 = 783.99;

// ---------------------------------------------------------------------------
// Timing and amplitude
// ---------------------------------------------------------------------------

/// Duration of each note in start/stop/yank chimes (seconds).
const NOTE_DURATION_SECS: f32 = 0.1;

/// Duration of flush/pause/resume/empty chime notes (seconds).
const SHORT_NOTE_DURATION_SECS: f32 = 0.08;

/// Chime playback amplitude (0.0 to 1.0).
const AMPLITUDE: f32 = 0.3;

/// Extra padding after chime playback to ensure it completes (seconds).
const PLAYBACK_PADDING_SECS: f32 = 0.05;

// ---------------------------------------------------------------------------
// Chime enum
// ---------------------------------------------------------------------------

/// Auditory feedback chimes for narration state transitions.
pub enum Chime {
    /// Rising C5→E5: recording started.
    Start,
    /// Descending E5→C5: recording stopped, content staged for delivery.
    Stop,
    /// Single G5: content flushed, still recording.
    Flush,
    /// D5: capture suspended.
    Pause,
    /// Rising D5→E5: capture resumed.
    Resume,
    /// Descending E5→A4: recording stopped, content copied to clipboard.
    Yank,
    /// A4: stop/flush/yank produced no content.
    Empty,
}

impl Chime {
    /// Render and play this chime. Errors are non-fatal (callers ignore them).
    ///
    /// No-op in test mode: avoids cpal audio device initialization and
    /// real `thread::sleep` that would bypass the mock clock.
    pub fn play(&self) -> anyhow::Result<()> {
        if crate::test_mode::is_active() {
            return Ok(());
        }
        let sr = output_sample_rate()?;
        let samples = match self {
            Chime::Start => two_notes(NOTE_C5, NOTE_E5, NOTE_DURATION_SECS, sr),
            Chime::Stop => two_notes(NOTE_E5, NOTE_C5, NOTE_DURATION_SECS, sr),
            Chime::Flush => render_note(NOTE_G5 as f64, SHORT_NOTE_DURATION_SECS, sr),
            Chime::Pause => render_note(NOTE_D5 as f64, SHORT_NOTE_DURATION_SECS, sr),
            Chime::Resume => two_notes(NOTE_D5, NOTE_E5, SHORT_NOTE_DURATION_SECS, sr),
            Chime::Yank => two_notes(NOTE_E5, NOTE_A4, NOTE_DURATION_SECS, sr),
            Chime::Empty => render_note(NOTE_A4 as f64, SHORT_NOTE_DURATION_SECS, sr),
        };
        play_buffer(&samples, sr)
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render a two-note chime sequence at the standard amplitude.
fn two_notes(freq1: f32, freq2: f32, duration_secs: f32, sample_rate: f64) -> Vec<f32> {
    let mut samples = render_note(freq1 as f64, duration_secs, sample_rate);
    samples.extend(render_note(freq2 as f64, duration_secs, sample_rate));
    samples
}

/// Render a single note with a sine-shaped amplitude envelope.
///
/// Uses a fundsp sine oscillator at the given frequency. The envelope
/// ramps smoothly from silence to peak and back (sin-shaped), avoiding
/// clicks at note boundaries.
fn render_note(freq: f64, duration_secs: f32, sample_rate: f64) -> Vec<f32> {
    use fundsp::prelude::*;

    let num_samples = (sample_rate * duration_secs as f64) as usize;
    let mut osc = sine_hz::<f64>(freq as f32);
    osc.set_sample_rate(sample_rate);
    osc.reset();

    (0..num_samples)
        .map(|i| {
            let pos = i as f64 / num_samples as f64;
            let envelope = (pos * std::f64::consts::PI).sin();
            osc.get_mono() as f32 * envelope as f32 * AMPLITUDE
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------

/// Query the default output device's sample rate.
fn output_sample_rate() -> anyhow::Result<f64> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no audio output device found"))?;
    let config = device.default_output_config()?;
    Ok(config.sample_rate() as f64)
}

/// Play a pre-rendered mono f32 buffer through the default output device.
fn play_buffer(samples: &[f32], sample_rate: f64) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no audio output device found"))?;

    let config = device.default_output_config()?;
    let channels = config.channels() as usize;
    let stream_config: cpal::StreamConfig = config.into();

    let buf = Arc::new(samples.to_vec());
    let idx = Arc::new(AtomicUsize::new(0));
    let idx_ref = Arc::clone(&idx);
    let buf_ref = Arc::clone(&buf);

    let stream = device.build_output_stream(
        &stream_config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let i = idx_ref.fetch_add(1, Ordering::Relaxed);
                let sample = buf_ref.get(i).copied().unwrap_or(0.0);
                for ch in frame.iter_mut() {
                    *ch = sample;
                }
            }
        },
        |err| tracing::error!("audio output error: {err}"),
        None,
    )?;

    stream.play()?;

    let duration_secs = samples.len() as f32 / sample_rate as f32 + PLAYBACK_PADDING_SECS;
    // Audio playback timing: not clock-gated because chime is a
    // no-op in test mode and doesn't participate in settlement.
    #[allow(clippy::disallowed_methods)]
    std::thread::sleep(std::time::Duration::from_secs_f32(duration_secs));

    Ok(())
}
