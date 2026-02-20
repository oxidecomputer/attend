# Phase 6: Recording Daemon Improvements

**Dependencies**: Phase 4 (nix for signals), Phase 5 (error handling patterns established).
**Effort**: Medium | **Risk**: Medium

---

## 6.1 Reorder daemon startup

- Current: preload model -> chime -> start capture
- New: start capture -> play chime -> preload model (on thread or lazily)
- Audio accumulates while model loads; user gets immediate feedback
- Block on model readiness only when first transcription is actually needed

## 6.2 Remove 200ms sleep in `spawn_daemon`

- The daemon's lock acquisition already prevents double-spawn
- Parent returns immediately; quick double-toggle races resolve via lock

## 6.3 Extract `DaemonState` struct

- Fields: `transcriber`, `capture`, `editor_events`, `silence_detector`, `buffered_chunks`, `pre_transcribed`, `period_start`, `time_base_secs`, `sample_rate`, `session_id`
- Methods: `ingest_chunks()`, `handle_stop()`, `handle_flush()`, `transcribe_and_write()`
- Main loop becomes: `state.ingest_chunks()?; if state.check_stop()? { break; } if state.check_flush()? { continue; } sleep(POLL_INTERVAL);`
- Eliminates `#[allow(clippy::too_many_arguments)]`

## 6.4 Signal handler for graceful lock cleanup

- Use `signal-hook` (already a dep) to catch SIGTERM
- Set a flag that the daemon loop checks, same as stop sentinel
- Ensures lock file is cleaned up even if process is killed externally

## 6.5 Add more commentary to audio and transcription logic

- Document `SincInterpolationParameters` choices in `audio.rs`
- Explain the chunk/padding strategy in `resample()`
- Document Whisper parameter choices in `whisper.rs` (why greedy, why max_len=1, why token_timestamps, etc.)

---

## Verification

- `grep -rn 'too_many_arguments' src/` returns zero hits
- Manual test: full recording lifecycle with `RUST_LOG=debug`:
  - `attend narrate toggle` -> chime plays immediately (not after model load delay)
  - Speak, pause, speak -> VAD log messages show correct transitions
  - `attend narrate stop` -> transcription completes, narration file written
- Manual test: `attend narrate toggle`, wait 30+ seconds idle, `attend narrate toggle` again -> starts cleanly (no stale lock)
- Manual test: kill daemon with `kill <pid>`, then `attend narrate toggle` -> starts cleanly (signal handler cleaned up lock, or stale lock detection works)
