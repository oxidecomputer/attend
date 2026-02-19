//! Microphone capture and resampling.
//!
//! Uses cpal for audio input, accumulating mono f32 samples with wall-clock
//! timestamps. After recording stops, resamples to 16 kHz via rubato for
//! Whisper consumption.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::json::utc_now;

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
    let sample_rate = config.sample_rate().0;
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
            eprintln!("audio capture error: {err}");
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

/// Resample a mono f32 buffer from `from_rate` to `to_rate` (typically 16000).
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> anyhow::Result<Vec<f32>> {
    if from_rate == to_rate {
        return Ok(samples.to_vec());
    }

    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let ratio = to_rate as f64 / from_rate as f64;
    let chunk_size = 1024;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, chunk_size, 1)?;

    let mut output = Vec::with_capacity((samples.len() as f64 * ratio) as usize + chunk_size);
    let mut pos = 0;

    while pos < samples.len() {
        let end = (pos + chunk_size).min(samples.len());
        let mut chunk = samples[pos..end].to_vec();
        if chunk.len() < chunk_size {
            chunk.resize(chunk_size, 0.0);
        }
        let result = resampler.process(&[&chunk], None)?;
        output.extend_from_slice(&result[0]);
        pos += chunk_size;
    }

    let expected = (samples.len() as f64 * ratio).ceil() as usize;
    output.truncate(expected);

    Ok(output)
}

/// Play a single short note as a flush chime (distinct from start/stop).
///
/// A single G5 note (~80ms) signals "submitted, still recording".
pub fn play_flush_chime() -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no audio output device found"))?;

    let config = device.default_output_config()?;
    let sample_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;

    let freq = 783.99_f32; // G5
    let note_samples = (sample_rate * 0.08) as usize; // 80ms
    let sample_idx = Arc::new(AtomicUsize::new(0));
    let sample_idx_ref = Arc::clone(&sample_idx);

    let stream_config: cpal::StreamConfig = config.into();

    let stream = device.build_output_stream(
        &stream_config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let idx = sample_idx_ref.fetch_add(1, Ordering::Relaxed);
                let sample = if idx >= note_samples {
                    0.0
                } else {
                    let t = idx as f32 / sample_rate;
                    let pos = idx as f32 / note_samples as f32;
                    let envelope = (pos * std::f32::consts::PI).sin();
                    (t * freq * 2.0 * std::f32::consts::PI).sin() * envelope * 0.3
                };
                for ch in frame.iter_mut() {
                    *ch = sample;
                }
            }
        },
        |err| {
            eprintln!("audio output error: {err}");
        },
        None,
    )?;

    stream.play()?;

    let total_duration =
        std::time::Duration::from_secs_f32(note_samples as f32 / sample_rate + 0.05);
    std::thread::sleep(total_duration);

    Ok(())
}

/// Play a short synthesized chime for auditory feedback.
///
/// `ascending`: true for start chime (low→high), false for stop (high→low).
pub fn play_chime(ascending: bool) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no audio output device found"))?;

    let config = device.default_output_config()?;
    let sample_rate = config.sample_rate().0 as f32;
    let channels = config.channels() as usize;

    // Two-note chime: 100ms per note
    let (freq1, freq2) = if ascending {
        (523.25_f32, 659.25_f32) // C5 → E5
    } else {
        (659.25_f32, 523.25_f32) // E5 → C5
    };

    let note_samples = (sample_rate * 0.1) as usize; // 100ms per note
    let total_samples = note_samples * 2;
    let sample_idx = Arc::new(AtomicUsize::new(0));
    let sample_idx_ref = Arc::clone(&sample_idx);

    let stream_config: cpal::StreamConfig = config.into();

    let stream = device.build_output_stream(
        &stream_config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let idx = sample_idx_ref.fetch_add(1, Ordering::Relaxed);
                let sample = if idx >= total_samples {
                    0.0
                } else {
                    let freq = if idx < note_samples { freq1 } else { freq2 };
                    let t = idx as f32 / sample_rate;
                    let envelope = if idx < note_samples {
                        let pos = idx as f32 / note_samples as f32;
                        (pos * std::f32::consts::PI).sin()
                    } else {
                        let pos = (idx - note_samples) as f32 / note_samples as f32;
                        (pos * std::f32::consts::PI).sin()
                    };
                    (t * freq * 2.0 * std::f32::consts::PI).sin() * envelope * 0.3
                };
                for ch in frame.iter_mut() {
                    *ch = sample;
                }
            }
        },
        |err| {
            eprintln!("audio output error: {err}");
        },
        None,
    )?;

    stream.play()?;

    // Wait for playback to complete
    let total_duration =
        std::time::Duration::from_secs_f32(total_samples as f32 / sample_rate + 0.05);
    std::thread::sleep(total_duration);

    Ok(())
}
