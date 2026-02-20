# Phase 11: Persistent Daemon

**Dependencies**: Phase 6 (daemon restructured), Phase 8 (UX improvements).
**Effort**: Medium-High | **Risk**: Medium

---

## Motivation

The daemon is currently spawned fresh for each recording session (`toggle` →
`spawn_daemon`). This means the whisper/parakeet model is loaded from disk into
memory on every activation. Model loading is not instant — it involves reading
hundreds of megabytes from disk, allocating GPU/CPU buffers, and initializing
inference state. This cost is paid every time the user starts narrating, adding
latency to the first transcription segment.

A persistent daemon that stays resident between sessions could amortize this
cost: load the model once, keep it warm, and serve multiple recording sessions
over its lifetime.

---

## 11.1 Benchmark model load time

- Instrument `Engine::preload` with wall-clock timing and log the result
- Measure on both whisper and parakeet backends
- Measure cold (first load after boot / page cache cold) vs warm (model file in page cache)
- This quantifies the actual cost and determines whether the rest of the phase is worth pursuing
- If load time is consistently <1s warm, document the finding and consider the phase complete

## 11.2 Design persistent daemon lifecycle

- Daemon stays alive after recording stops, rather than exiting
- New states: `Idle` (model loaded, no audio capture) vs `Recording` (current behavior)
- Idle timeout: configurable duration (default e.g. 5 minutes) after which daemon exits to reclaim memory
- Config field: `daemon_idle_timeout = "5m"` (or `"forever"` to never auto-exit)
- On next `toggle`/`start`: detect running daemon via lock file, send "start recording" signal instead of spawning

## 11.3 IPC upgrade: sentinel files → command channel

- Current sentinel-file IPC (stop, flush) is simple but one-shot: daemon exits after handling stop
- Persistent daemon needs bidirectional commands: start, stop, flush, shutdown, status
- Evaluate options:
  - Unix domain socket (most flexible, bidirectional, well-supported by `nix` crate)
  - Named pipe / FIFO (simpler, one-directional per pipe)
  - Extend sentinel files with a command vocabulary (simplest, least capable)
- Recommendation: Unix domain socket in `cache_dir/daemon.sock`
- Keep sentinel files as a fallback for emergency stop (kill -TERM + sentinel) for robustness

## 11.4 Implement idle state

- After stop: tear down `audio_capture` and `editor_capture`, keep `transcriber` loaded
- Reset `DaemonState` fields (buffered_chunks, pre_transcribed, timing) without dropping transcriber
- Daemon enters idle loop: poll command channel + idle timeout
- On "start recording" command: reinitialize audio/editor capture, resume recording loop
- On idle timeout expiry: clean exit (release lock, remove socket)

## 11.5 Memory footprint analysis

- Measure RSS in idle state (model loaded, no audio capture) vs during recording
- Whisper small.en model is ~500MB on disk; resident memory may differ
- Parakeet TDT 0.6B similarly large
- Document findings; if idle RSS is problematic, consider:
  - Model memory-mapping (mmap) so OS can page out under pressure
  - Explicit model unload after extended idle, with reload on next activation

## 11.6 Warm-start correctness

- Verify WhisperState/initial_prompt carry-over across sessions is desirable
  - Pro: context from previous narration could improve transcription of follow-up
  - Con: stale context from unrelated earlier session could confuse the model
- Likely: reset `initial_prompt` on each new recording start, keep model weights warm
- Verify parakeet backend handles the same lifecycle correctly

## 11.7 Daemon health and observability

- `attend narrate status` should report: daemon PID, state (idle/recording), uptime, model loaded, memory usage
- Useful for debugging and for agents to know if the daemon is healthy without starting a recording
- Consider a heartbeat or health-check mechanism so stale lock files from crashed daemons are detected

---

## Verification

- Benchmark (11.1): log output shows model load time; compare against "time to first transcription" in current design
- Integration test (11.2, 11.4): toggle on → narrate → toggle off → toggle on again → narrate; verify second activation is near-instant (no model reload)
- Manual test (11.3): `attend narrate status` shows daemon state; flush and stop work via new IPC
- Manual test (11.4): start recording, stop, wait past idle timeout → daemon exits cleanly, lock file removed
- Memory test (11.5): record RSS via `ps` at idle vs recording; document in findings
- Correctness test (11.6): two back-to-back narration sessions; verify transcription quality of second session is not degraded by stale context
