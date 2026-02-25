use super::*;

/// Resampling from rate X to rate X must return an exact copy of the input.
#[test]
fn identity_resample_returns_same_samples() {
    let input: Vec<f32> = (0..1000).map(|i| (i as f32) * 0.001).collect();
    let output = resample(&input, 48000, 48000).unwrap();
    assert_eq!(input, output);
}

/// Downsampling from 48 kHz to 16 kHz should produce output whose length
/// matches the expected ratio (input_len * 16000 / 48000) within ±2
/// samples to account for edge effects in the sinc resampler.
#[test]
fn output_length_matches_ratio() {
    let input_len = 48000; // 1 second at 48 kHz
    let input: Vec<f32> = vec![0.0; input_len];
    let output = resample(&input, 48000, 16000).unwrap();

    let expected = (input_len as f64 * 16000.0 / 48000.0).ceil() as usize;
    let diff = (output.len() as isize - expected as isize).unsigned_abs();
    assert!(
        diff <= 2,
        "output length {} differs from expected {} by {} (tolerance: 2)",
        output.len(),
        expected,
        diff,
    );
}

/// A 440 Hz sine wave generated at 48 kHz, resampled to 16 kHz, should
/// preserve its dominant frequency. We verify by counting zero-crossings
/// in the resampled signal: a 440 Hz tone has ~880 zero-crossings per
/// second (two per cycle). We allow 5% tolerance.
#[test]
fn sine_preservation_via_zero_crossings() {
    let from_rate = 48000u32;
    let to_rate = 16000u32;
    let duration_secs = 0.5;
    let freq_hz = 440.0f32;

    let num_samples = (from_rate as f32 * duration_secs) as usize;
    let input: Vec<f32> = (0..num_samples)
        .map(|i| {
            let t = i as f32 / from_rate as f32;
            (2.0 * std::f32::consts::PI * freq_hz * t).sin()
        })
        .collect();

    let output = resample(&input, from_rate, to_rate).unwrap();

    // Count zero-crossings: where consecutive samples have opposite sign.
    // Skip a small prefix to avoid resampler transient effects.
    let skip = (to_rate as f32 * 0.02) as usize; // skip first 20ms
    let analysis = &output[skip..];
    let zero_crossings = analysis
        .windows(2)
        .filter(|w| w[0].signum() != w[1].signum())
        .count();

    let analysis_duration = analysis.len() as f64 / to_rate as f64;
    let crossings_per_sec = zero_crossings as f64 / analysis_duration;

    // A pure sine at F Hz has 2*F zero-crossings per second.
    let expected_crossings = 2.0 * freq_hz as f64;
    let tolerance = expected_crossings * 0.05;
    let diff = (crossings_per_sec - expected_crossings).abs();
    assert!(
        diff <= tolerance,
        "zero-crossings/sec {crossings_per_sec:.1} differs from expected \
             {expected_crossings:.1} by {diff:.1} (tolerance: {tolerance:.1})",
    );
}

/// A constant-valued (DC) signal should remain approximately constant
/// after resampling. The mean of the output should be within 0.01 of
/// the original DC value.
#[test]
fn dc_preservation() {
    let dc_value = 0.5f32;
    let input: Vec<f32> = vec![dc_value; 48000]; // 1 second at 48 kHz
    let output = resample(&input, 48000, 16000).unwrap();

    assert!(!output.is_empty(), "output should not be empty");

    // Skip a small prefix/suffix to avoid edge transients from the sinc
    // resampler's ramp-up and the final zero-padded chunk.
    let skip = 512;
    let analysis = &output[skip..output.len().saturating_sub(skip)];
    assert!(
        !analysis.is_empty(),
        "analysis slice should not be empty after trimming edges",
    );

    let mean = analysis.iter().copied().sum::<f32>() / analysis.len() as f32;
    let diff = (mean - dc_value).abs();
    assert!(
        diff <= 0.01,
        "mean {mean:.4} differs from DC value {dc_value} by {diff:.4} (tolerance: 0.01)",
    );
}

/// Resampling an empty input slice should return an empty Vec without error.
#[test]
fn empty_input_returns_empty_output() {
    let output = resample(&[], 48000, 16000).unwrap();
    assert!(
        output.is_empty(),
        "expected empty output, got {} samples",
        output.len()
    );
}
