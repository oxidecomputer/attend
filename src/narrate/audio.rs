//! Microphone capture and resampling.
//!
//! Uses cpal for audio input, accumulating mono f32 samples with wall-clock
//! timestamps. After recording stops, resamples to 16 kHz via rubato for
//! Whisper consumption.

use std::sync::{Arc, Mutex};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Resampler constants (rubato sinc interpolation)
// ---------------------------------------------------------------------------
//
// We use rubato's async sinc resampler to downsample from the device's native
// rate (typically 44.1/48 kHz) to 16 kHz for transcription. The "async" variant
// handles non-integer ratios (e.g. 48000/16000 = 3, but 44100/16000 = 2.75625).
//
// Sinc interpolation with a BlackmanHarris window gives excellent alias
// rejection. The parameters below trade off quality vs. CPU: a longer kernel
// and higher oversampling improve frequency response at the cost of more
// multiplies per sample. These values are generous for speech (which only
// occupies 0-8 kHz) but cheap enough on modern hardware.

/// Sinc interpolation kernel length (number of zero-crossings on each side).
/// Longer = sharper cutoff but more computation. 256 is high-quality; speech
/// would be fine with 64-128, but the extra cost is negligible for our
/// throughput (~5 seconds of audio at a time).
const RESAMPLER_SINC_LEN: usize = 256;

/// Low-pass cutoff frequency as a fraction of the output Nyquist frequency.
/// 0.95 preserves nearly all energy below Nyquist while still leaving a
/// small transition band for the anti-aliasing filter to roll off.
const RESAMPLER_CUTOFF: f32 = 0.95;

/// Sinc table oversampling factor. The resampler precomputes a table of sinc
/// values and interpolates between entries. Higher = more accurate interpolation
/// at the cost of memory (256 * 256 * sizeof(f32) = 256 KB, trivial).
const RESAMPLER_OVERSAMPLING: usize = 256;

/// Number of input samples processed per resampler call. Larger batches
/// amortize per-call overhead. 1024 is a reasonable default; the last chunk
/// is zero-padded if shorter (see `resample()`).
const RESAMPLER_CHUNK_SIZE: usize = 1024;

/// Transition bandwidth parameter for the anti-aliasing filter design.
/// Controls how quickly the filter rolls off above the cutoff. A value of
/// 2.0 gives a moderate transition band — wider than the minimum (which
/// would require a longer kernel) but sufficient for speech.
const RESAMPLER_TRANSITION_BW: f64 = 2.0;

/// A timestamped chunk of audio samples.
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Monotonic instant for relative timing.
    pub instant: Instant,
    /// Mono f32 samples at the device's native sample rate.
    pub samples: Vec<f32>,
}

/// Accumulated recording data.
#[derive(Debug)]
pub struct Recording {
    /// All captured chunks in chronological order.
    pub chunks: Vec<AudioChunk>,
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
    })
}

/// Handle to an in-progress audio capture.
pub struct CaptureHandle {
    stream: cpal::Stream,
    chunks: Arc<Mutex<Vec<AudioChunk>>>,
    sample_rate: u32,
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

        Recording { chunks }
    }

    /// Pause the audio stream.
    ///
    /// Stops the cpal callback, saving CPU/power and on macOS turning off the
    /// mic indicator dot. Chunks already in the buffer are preserved.
    pub fn pause(&self) -> anyhow::Result<()> {
        use cpal::traits::StreamTrait;
        self.stream.pause()?;
        Ok(())
    }

    /// Resume the audio stream after a pause.
    pub fn resume(&self) -> anyhow::Result<()> {
        use cpal::traits::StreamTrait;
        self.stream.play()?;
        Ok(())
    }

    /// Stop recording and return the accumulated audio data.
    pub fn stop(self) -> Recording {
        drop(self.stream);

        let chunks = match Arc::try_unwrap(self.chunks) {
            Ok(mutex) => mutex.into_inner().unwrap_or_default(),
            Err(arc) => arc.lock().unwrap().clone(),
        };

        Recording { chunks }
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

/// Resample a mono f32 buffer from `from_rate` to `to_rate` (typically 16 kHz).
///
/// Processes the input in fixed-size chunks (`RESAMPLER_CHUNK_SIZE` samples).
/// The final chunk is zero-padded to the full chunk size because rubato
/// requires fixed-length input buffers. After processing, the output is
/// truncated to the mathematically expected sample count to remove any
/// trailing silence introduced by padding.
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
        // Linear interpolation between sinc table entries. Cubic would give
        // marginally better frequency response but costs more per sample.
        // For speech destined for a neural transcription model, the difference
        // is inaudible and immeasurable.
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: RESAMPLER_OVERSAMPLING,
        // BlackmanHarris2 has excellent sidelobe suppression (~92 dB), keeping
        // aliased energy well below the noise floor of typical microphone input.
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

#[cfg(test)]
mod tests;
