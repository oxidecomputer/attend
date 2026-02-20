//! Microphone capture and resampling.
//!
//! Uses cpal for audio input, accumulating mono f32 samples with wall-clock
//! timestamps. After recording stops, resamples to 16 kHz via rubato for
//! Whisper consumption.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::util::utc_now;

// ---------------------------------------------------------------------------
// Chime constants
// ---------------------------------------------------------------------------

/// Frequency for flush chime: G5.
const FLUSH_CHIME_FREQ_HZ: f32 = 783.99;

/// Duration of the flush chime note (seconds).
const FLUSH_CHIME_DURATION_SECS: f32 = 0.08;

/// Start chime note 1: C5.
const CHIME_NOTE_C5_HZ: f32 = 523.25;

/// Start chime note 2: E5.
const CHIME_NOTE_E5_HZ: f32 = 659.25;

/// Duration of each note in start/stop chime (seconds).
const CHIME_NOTE_DURATION_SECS: f32 = 0.1;

/// Chime playback amplitude (0.0 to 1.0).
const CHIME_AMPLITUDE: f32 = 0.3;

/// Extra padding after chime playback to ensure it completes (seconds).
const CHIME_PLAYBACK_PADDING_SECS: f32 = 0.05;

// ---------------------------------------------------------------------------
// Resampler constants (rubato sinc interpolation)
// ---------------------------------------------------------------------------

/// Sinc interpolation kernel length.
const RESAMPLER_SINC_LEN: usize = 256;

/// Low-pass cutoff frequency (fraction of Nyquist).
const RESAMPLER_CUTOFF: f32 = 0.95;

/// Sinc oversampling factor for table lookup.
const RESAMPLER_OVERSAMPLING: usize = 256;

/// Number of input samples processed per resampler call.
const RESAMPLER_CHUNK_SIZE: usize = 1024;

/// Transition bandwidth for the anti-aliasing filter.
const RESAMPLER_TRANSITION_BW: f64 = 2.0;

/// A timestamped chunk of audio samples.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AudioChunk {
    /// Wall-clock ISO 8601 timestamp when this chunk was captured.
    pub wall_clock: String,
    /// Monotonic instant for relative timing.
    pub instant: Instant,
    /// Mono f32 samples at the device's native sample rate.
    pub samples: Vec<f32>,
}

/// Accumulated recording data.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Recording {
    /// All captured chunks in chronological order.
    pub chunks: Vec<AudioChunk>,
    /// Native sample rate of the input device.
    pub sample_rate: u32,
    /// Monotonic instant when recording started.
    pub start_instant: Instant,
    /// Wall-clock timestamp when recording started.
    pub start_wall_clock: String,
}

#[allow(dead_code)]
impl Recording {
    /// Total number of samples across all chunks.
    pub fn total_samples(&self) -> usize {
        self.chunks.iter().map(|c| c.samples.len()).sum()
    }

    /// Total duration in seconds based on sample count and rate.
    pub fn duration_secs(&self) -> f64 {
        self.total_samples() as f64 / self.sample_rate as f64
    }

    /// Concatenate all chunks into a single f32 buffer.
    pub fn flatten(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.total_samples());
        for chunk in &self.chunks {
            out.extend_from_slice(&chunk.samples);
        }
        out
    }

    /// Wall-clock time offset (in seconds from recording start) for a given
    /// sample index.
    #[allow(dead_code)]
    pub fn sample_to_offset_secs(&self, sample_idx: usize) -> f64 {
        sample_idx as f64 / self.sample_rate as f64
    }
}

/// Start capturing audio from the default input device.
///
/// Returns a handle that accumulates samples in the background.
/// Call `stop()` on the handle to finish recording and get the `Recording`.
pub fn start_capture() -> anyhow::Result<CaptureHandle> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no audio input device found"))?;

    let config = device.default_input_config()?;
    let sample_rate = config.sample_rate();
    let channels = config.channels() as usize;

    let chunks: Arc<Mutex<Vec<AudioChunk>>> = Arc::new(Mutex::new(Vec::new()));
    let chunks_ref = Arc::clone(&chunks);
    let start_instant = Instant::now();
    let start_wall_clock = utc_now();

    let stream_config: cpal::StreamConfig = config.into();

    let stream = device.build_input_stream(
        &stream_config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // Downmix to mono by averaging channels
            let mono: Vec<f32> = data
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect();

            let chunk = AudioChunk {
                wall_clock: utc_now(),
                instant: Instant::now(),
                samples: mono,
            };

            if let Ok(mut guard) = chunks_ref.lock() {
                guard.push(chunk);
            }
        },
        |err| {
            tracing::error!("audio capture error: {err}");
        },
        None,
    )?;

    stream.play()?;

    Ok(CaptureHandle {
        stream,
        chunks,
        sample_rate,
        start_instant,
        start_wall_clock,
    })
}

/// Handle to an in-progress audio capture.
pub struct CaptureHandle {
    stream: cpal::Stream,
    chunks: Arc<Mutex<Vec<AudioChunk>>>,
    sample_rate: u32,
    start_instant: Instant,
    start_wall_clock: String,
}

impl CaptureHandle {
    /// Take all accumulated chunks, leaving the buffer empty.
    ///
    /// The cpal stream keeps running — new samples accumulate into the
    /// now-empty Vec. Use this for incremental processing during recording.
    pub fn take_chunks(&self) -> Vec<AudioChunk> {
        std::mem::take(&mut *self.chunks.lock().unwrap())
    }

    /// The native sample rate of the input device.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Drain accumulated audio without stopping the capture stream.
    ///
    /// Returns a `Recording` of everything captured so far, then resets the
    /// internal buffer. The cpal stream keeps running — new samples accumulate
    /// into the now-empty Vec. No audio is lost.
    pub fn drain(&self) -> Recording {
        let chunks = std::mem::take(&mut *self.chunks.lock().unwrap());

        Recording {
            chunks,
            sample_rate: self.sample_rate,
            start_instant: self.start_instant,
            start_wall_clock: self.start_wall_clock.clone(),
        }
    }

    /// Stop recording and return the accumulated audio data.
    pub fn stop(self) -> Recording {
        drop(self.stream);

        let chunks = match Arc::try_unwrap(self.chunks) {
            Ok(mutex) => mutex.into_inner().unwrap_or_default(),
            Err(arc) => arc.lock().unwrap().clone(),
        };

        Recording {
            chunks,
            sample_rate: self.sample_rate,
            start_instant: self.start_instant,
            start_wall_clock: self.start_wall_clock,
        }
    }
}

/// Concatenate audio chunks into a single f32 buffer.
pub fn flatten_chunks(chunks: &[AudioChunk]) -> Vec<f32> {
    let total: usize = chunks.iter().map(|c| c.samples.len()).sum();
    let mut out = Vec::with_capacity(total);
    for chunk in chunks {
        out.extend_from_slice(&chunk.samples);
    }
    out
}

/// Resample a mono f32 buffer from `from_rate` to `to_rate` (typically 16000).
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> anyhow::Result<Vec<f32>> {
    if from_rate == to_rate {
        return Ok(samples.to_vec());
    }

    use audioadapter_buffers::direct::InterleavedSlice;
    use rubato::{
        Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
        WindowFunction,
    };

    let params = SincInterpolationParameters {
        sinc_len: RESAMPLER_SINC_LEN,
        f_cutoff: RESAMPLER_CUTOFF,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: RESAMPLER_OVERSAMPLING,
        window: WindowFunction::BlackmanHarris2,
    };

    let ratio = to_rate as f64 / from_rate as f64;
    let mut resampler = Async::<f32>::new_sinc(
        ratio,
        RESAMPLER_TRANSITION_BW,
        &params,
        RESAMPLER_CHUNK_SIZE,
        1,
        FixedAsync::Input,
    )?;

    let mut output =
        Vec::with_capacity((samples.len() as f64 * ratio) as usize + RESAMPLER_CHUNK_SIZE);
    let mut pos = 0;

    while pos < samples.len() {
        let end = (pos + RESAMPLER_CHUNK_SIZE).min(samples.len());
        let mut chunk = samples[pos..end].to_vec();
        if chunk.len() < RESAMPLER_CHUNK_SIZE {
            chunk.resize(RESAMPLER_CHUNK_SIZE, 0.0);
        }
        let input = InterleavedSlice::new(&chunk, 1, RESAMPLER_CHUNK_SIZE)?;
        let result = resampler.process(&input, 0, None)?;
        output.extend(result.take_data());
        pos += RESAMPLER_CHUNK_SIZE;
    }

    let expected = (samples.len() as f64 * ratio).ceil() as usize;
    output.truncate(expected);

    Ok(output)
}

/// Play a single short note as a flush chime (distinct from start/stop).
///
/// A single G5 note (~80ms) signals "submitted, still recording".
pub fn play_flush_chime() -> anyhow::Result<()> {
    let sample_rate = output_sample_rate()?;
    let samples = render_note(
        FLUSH_CHIME_FREQ_HZ as f64,
        FLUSH_CHIME_DURATION_SECS,
        sample_rate,
    );
    play_buffer(&samples, sample_rate)
}

/// Play a short synthesized chime for auditory feedback.
///
/// `ascending`: true for start chime (low->high), false for stop (high->low).
pub fn play_chime(ascending: bool) -> anyhow::Result<()> {
    let sample_rate = output_sample_rate()?;

    // Two-note chime: C5→E5 (start) or E5→C5 (stop).
    let (freq1, freq2) = if ascending {
        (CHIME_NOTE_C5_HZ, CHIME_NOTE_E5_HZ)
    } else {
        (CHIME_NOTE_E5_HZ, CHIME_NOTE_C5_HZ)
    };

    let mut samples = render_note(freq1 as f64, CHIME_NOTE_DURATION_SECS, sample_rate);
    samples.extend(render_note(
        freq2 as f64,
        CHIME_NOTE_DURATION_SECS,
        sample_rate,
    ));
    play_buffer(&samples, sample_rate)
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
            osc.get_mono() as f32 * envelope as f32 * CHIME_AMPLITUDE
        })
        .collect()
}

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

    let duration_secs = samples.len() as f32 / sample_rate as f32 + CHIME_PLAYBACK_PADDING_SECS;
    std::thread::sleep(std::time::Duration::from_secs_f32(duration_secs));

    Ok(())
}
