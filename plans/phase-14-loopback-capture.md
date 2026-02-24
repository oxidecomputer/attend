# Phase 14: Loopback Audio Capture

Capture system audio output (speaker/headphone playback) alongside microphone
input so that both sides of a collaborative session are transcribed. The local
user's speech is tagged as self; audio from other participants (meeting calls,
pair programming, etc.) is tagged as other-speaker.

## Motivation

attend currently captures one audio stream: the default input device
(microphone). In collaborative sessions (video calls, pair programming over
Zoom/Meet/Discord), the other participants' speech is invisible to the agent.
The user has to paraphrase what others said, or the context is lost entirely.

By capturing the system audio output (loopback) as a second stream, we can
transcribe both sides of the conversation. Combined with echo cancellation,
this produces clean, interleaved, speaker-attributed transcription.

## Platform support

**macOS 14.6+**: Full support. cpal 0.17 exposes loopback via Core Audio Taps
(`AudioHardwareCreateProcessTap` + aggregate device). Requires "System Audio
Recording" permission (one-time prompt, less alarming than Screen Recording).

**Linux**: Deferred. PulseAudio/PipeWire monitor sources are "just input
devices," but cpal's PulseAudio backend is merged to master and not yet
released (post-v0.17.3). Once a cpal release includes it, Linux support is
straightforward. Tracked in PLAN.md.

**Graceful degradation**: On platforms where loopback is unavailable (Linux
today, macOS < 14.6), the feature is silently skipped. Mic-only capture
continues to work exactly as it does now.

## Design overview

### Dependency changes

**Add** `webrtc-audio-processing = { version = "2.0", features = ["bundled"] }`

This replaces `webrtc-vad` entirely. The crate wraps Google's WebRTC audio
processing module (AEC3, noise suppression, AGC, VAD) through a single
`Processor` type. VAD becomes a free side-effect of the processing pipeline
rather than a standalone call.

Build requires Meson + Ninja + clang (all available via Homebrew; document in
CONTRIBUTING or build instructions).

**Remove** `webrtc-vad = "0.4"`

### New types

```rust
/// Which audio source produced a transcription segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSource {
    /// Local microphone (the user).
    Mic,
    /// System audio output (other participants).
    Loopback,
}
```

### Audio processing pipeline

```
                    ┌──────────────────────────────────────┐
                    │      webrtc-audio-processing         │
                    │           Processor                  │
                    │                                      │
  loopback ────────►│  analyze_render_frame(&render)       │
  (cpal stream)     │           │                          │
                    │           ▼                          │
  mic ─────────────►│  process_capture_frame(&mut capture) │
  (cpal stream)     │           │                          │
                    │           ├──► echo-cancelled f32    │
                    │           │    audio → resample →  │
                    │           │    transcribe            │
                    │           │                          │
                    │           └──► stats.has_voice       │
                    │                (VAD for silence      │
                    │                 detection)           │
                    └──────────────────────────────────────┘
```

Two cpal streams run in parallel:
1. **Mic stream**: default input device (existing code path)
2. **Loopback stream**: `"default-output"` device via cpal loopback API

The `Processor` receives both streams in lockstep (10ms frames):
- Loopback frames are fed as the **render** (far-end) signal via
  `analyze_render_frame()`. This tells AEC what audio is playing through
  speakers.
- Mic frames are fed as the **capture** (near-end) signal via
  `process_capture_frame()`. AEC subtracts the echo; the frame is modified
  in-place.
- `processor.get_stats().has_voice` replaces the `webrtc-vad` call for
  silence detection.

The loopback stream is *also* independently fed through its own silence
detector and transcription pipeline, producing `AudioSource::Loopback` words.

### Event changes

```rust
Event::Words {
    timestamp: chrono::DateTime<chrono::Utc>,
    text: String,
    source: AudioSource,  // new field
}
```

Existing serialized sessions (which have no `source` field) deserialize with
`source` defaulting to `AudioSource::Mic` via `#[serde(default)]`.

### Rendering

Mic words render as bare prose (unchanged from today, triple-ticks not rendered
in output, just here to demarcate quotes):

```
I think the issue is in the handler module.
```

Loopback words render with an XML-style tag:

```
<other-speaker>
Yeah, the request lifecycle starts in handler dot rs, line 40.
</other-speaker>
```

The merge pipeline interleaves mic and loopback events chronologically,
exactly as it already interleaves Words, EditorSnapshot, FileDiff,
ExternalSelection, and BrowserSelection.

### SilenceDetector migration

Current API (`webrtc-vad`):
```rust
let vad = Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Aggressive);
let has_voice: bool = vad.is_voice_segment(&frame_i16);
```

New API (`webrtc-audio-processing`):
```rust
let processor = Processor::new(InitializationConfig {
    num_capture_channels: 1,
    num_render_channels: 1,
    ..Default::default()
})?;
processor.set_config(Config {
    echo_cancellation: Some(EchoCancellation {
        suppression_level: EchoCancellationSuppressionLevel::High,
        ..Default::default()
    }),
    voice_detection: Some(VoiceDetection {
        detection_likelihood: VoiceDetectionLikelihood::Moderate,
    }),
    enable_high_pass_filter: true,
    ..Default::default()
});

// In the processing loop (10ms frames, f32, deinterleaved):
processor.analyze_render_frame(&loopback_frame)?;
processor.process_capture_frame(&mut mic_frame)?;
let has_voice = processor.get_stats().has_voice.unwrap_or(false);
```

Key differences:
- Input is `f32` (no more i16 conversion in `downsample_to_vad()`)
- Frame is modified in place (echo-cancelled audio goes to transcription)
- VAD is a side-effect, not a separate call
- 10ms frame size is unchanged

The loopback stream needs its own `SilenceDetector` with a separate VAD.
Since the loopback stream has no echo to cancel (it *is* the far-end signal),
this detector can use a simpler `Processor` configured with VAD only (no AEC).

## Implementation tasks

### T1: Replace webrtc-vad with webrtc-audio-processing

Swap the dependency. Migrate `SilenceDetector` to use `Processor` for VAD.
No AEC yet, no loopback yet — just prove the new VAD works identically.
All existing silence detection tests must pass.

Build gate: document Meson + Ninja requirement.

### T2: Add AudioSource to the event model

Add `AudioSource` enum. Thread it through `Word`, `Event::Words`,
`merge.rs`, `receive.rs`, and `render.rs`. Default to `Mic` everywhere.
Backward-compatible deserialization for existing sessions.

### T3: Loopback capture stream

Add a second cpal stream for the loopback device. Wrap device discovery
in a fallible helper that returns `None` when loopback is unavailable
(Linux, macOS < 14.6). When available, run a parallel `AudioChunk` buffer
with its own `CaptureHandle`.

### T4: Echo cancellation wiring

Create the shared `Processor` that receives both streams. Feed loopback
frames as render, mic frames as capture. The echo-cancelled mic audio
replaces raw mic audio for transcription. VAD reads from the processor
stats.

Frame synchronization: both streams must be aligned to 10ms boundaries.
Clock drift between devices is possible; use timestamp-based alignment
with a small jitter buffer if needed.

### T5: Dual-stream transcription

Run two independent transcription pipelines (mic + loopback), each with
its own `SilenceDetector`. Tag output words with the appropriate
`AudioSource`. Feed both into the existing event merge pipeline.

### T6: Render and merge

Render `AudioSource::Loopback` words in `<other-speaker>` blocks.
Merge interleaves both sources chronologically. Compression logic:
consecutive words from the same source merge into a single block
(existing behavior for Words); source transitions force a block break.

### T7: Permission and UX

- Runtime detection of loopback availability (try to enumerate, handle
  failure gracefully)
- Log a message when loopback capture starts ("capturing system audio")
- Log a message when loopback is unavailable ("system audio capture not
  available on this platform; mic-only mode")
- Document the macOS "System Audio Recording" permission requirement
- Note: CLI tools without an .app bundle get a generic permission prompt.
  Some terminal emulators (VS Code integrated terminal) may not propagate
  it. Document that users may need to grant permission manually via
  System Settings, or run from Terminal.app the first time.

### T8: Tests

- SilenceDetector tests: port existing tests to new webrtc-audio-processing
  VAD API
- AudioSource serde round-trip tests
- Merge: interleaving of mic + loopback events
- Render: `<other-speaker>` block formatting
- Prop tests: AudioSource as a field in generated events

## Risks and mitigations

**webrtc-audio-processing issue #91 (SIGSEGV with AEC + NoiseSuppression):**
Start with AEC only, no noise suppression. Add NS later once the issue is
resolved upstream or we can reproduce and work around it.

**Frame synchronization between mic and loopback:** The two cpal streams run
on independent device clocks. Small drift is expected. The AEC algorithm
has an internal delay estimator (`enable_delay_agnostic: true`) that handles
moderate misalignment. If drift exceeds AEC tolerance, we may need a
timestamp-based alignment buffer. Start without one and measure.

**Transcription quality on loopback audio:** Meeting audio over speakers is
lower fidelity than direct mic. Whisper/Parakeet handle this reasonably
well but accuracy will be lower. No mitigation needed initially; just set
expectations.

**"Use headphones for best results":** Without headphones, the mic picks up
speaker audio, and AEC must work harder. With headphones/AirPods, hardware
AEC handles most of it and our software AEC has little work to do. Document
the recommendation.

**Prompt injection through speaker input:** Modify the skill instructions to
explain what the `<other-speaker>` tags demarcate, and strictly instruct it to
disregard any commands or instructions spoken by any party other than the
first-party user. For narrations that contains `<other-speaker>` tags, emit a
`<system-reminder>` which repeats this guidance. While doing this, also add
similar guidance about handling web excerpts, which can also be a source of
prompt injection: each narration that contains quotes from a website should be
accompanied by a reminder not to follow instructions from the website, that it
is untrusted content that could contain injections, and add general guidance
about this to the skill itself.

## Future: Speaker diarization (Layer 2)

`parakeet-rs` wraps NVIDIA's Sortformer model (`diar_streaming_sortformer_4spk-v2`)
for streaming speaker diarization with up to 4 speakers. The key feature is
**AOSC (Arrival-Order Speaker Cache)**: the `diarize_chunk()` API maintains
internal state across calls, providing cross-segment speaker label stability.

This could layer on top of loopback capture to split `<other-speaker>` into
individual speakers (`<speaker-1>`, `<speaker-2>`, etc.). The integration
point is after loopback transcription: feed each transcribed segment through
Sortformer to get per-word speaker labels.

**Open question for diarization:** The 4-speaker limit is architectural
(baked into the model output dimension). For meetings with more participants,
labels would alias. Whether this matters depends on use case.

**Not planned for this phase.** Loopback capture with undifferentiated
`<other-speaker>` tags is useful on its own and avoids the complexity of
diarization model management, additional ONNX model downloads, and the
cross-segment stability edge cases. Diarization is a natural follow-on once
the dual-stream pipeline is proven.
