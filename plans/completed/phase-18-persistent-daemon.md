# Phase 18: Persistent Daemon

**Dependencies**: Phase 14 (pause — provides full capture suspend/resume).
**Effort**: Medium | **Risk**: Low-Medium

---

## Motivation

The daemon is currently spawned fresh for each recording session (`toggle` →
`spawn_daemon`). This means the whisper/parakeet model is loaded from disk into
memory on every activation. Model loading involves reading hundreds of
megabytes from disk, allocating CPU buffers, and initializing inference state.
This cost is paid every time the user starts narrating.

Phase 14 introduces pause, which fully suspends all capture (cpal stream
paused, editor/diff/ext threads sleeping) with near-zero resource usage while
keeping the daemon process alive and the transcription model loaded. This is
most of the machinery needed for a persistent daemon: **stop becomes
flush+pause instead of flush+exit**, and the daemon stays resident for instant
restart.

What remains for phase 18 is:
1. Making the daemon survive across stop/start cycles (don't exit on stop).
2. Adding an idle timeout so the daemon eventually exits if unused.
3. Upgrading IPC so `toggle`/`start` can wake a paused daemon instead of
   spawning a new one.
4. Health and observability.

---

## 11.1 Benchmark model load time

- Instrument `Engine::preload` with wall-clock timing and log the result.
- Measure on both whisper and parakeet backends.
- Measure cold (first load after boot / page cache cold) vs warm (model file
  in page cache).
- This quantifies the actual cost and confirms the phase is worth pursuing.
- If load time is consistently <1s warm, document the finding and consider
  whether the phase still adds value (it does for session continuity, even
  if latency is low).

## 11.2 Stop → flush+pause

- Change `check_stop()` to flush accumulated content and enter the paused
  state (from phase 14) instead of exiting the daemon.
- The daemon loop continues running after stop: polling sentinels, idle
  timeout, etc.
- The record lock is **retained** while the daemon is alive (idle or
  recording). This is how `toggle`/`start` detect a running daemon.
- The stop sentinel is acknowledged and deleted as before.
- Transcriber state (`initial_prompt` / context) is reset on stop so stale
  context from the previous session doesn't affect the next one. Model
  weights stay loaded.

## 11.3 IPC: wake a paused daemon

- Current flow: `toggle`/`start` check `record_lock_path()`. If absent,
  spawn a new daemon. If present, send stop/flush sentinels.
- New flow: If lock is present and daemon is idle (paused), send a **resume
  sentinel** to wake it. The daemon resumes capture (cpal `play()`, unpause
  threads) and enters the recording state.
- The resume sentinel from phase 14 already exists. `start()` writes it when
  the daemon is paused; `spawn_daemon()` is only called when no daemon is
  running at all.
- Distinguish "recording" from "idle" for `toggle` semantics:
  - **Lock present + pause sentinel absent** → recording → send stop.
  - **Lock present + pause sentinel present** → idle → delete pause sentinel
    (resume).
  - **Lock absent** → spawn new daemon.
- This keeps sentinel-file IPC. No Unix domain socket needed at this stage.
  The sentinel vocabulary is: `stop`, `flush`, `pause` (presence = idle),
  `yank`. All one-shot except `pause` which is state.

## 11.4 Idle timeout

- While in idle (paused) state, the daemon tracks wall-clock time since
  entering idle.
- Config field: `daemon_idle_timeout` (default `"5m"`, or `"forever"` to
  never auto-exit). Parsed with `humantime`.
- When idle duration exceeds the timeout: clean exit (release lock, log).
- The idle timeout check lives in the daemon main loop alongside sentinel
  polling (100ms cadence is sufficient).

## 11.5 Memory footprint analysis

- Measure RSS in idle state (model loaded, all capture suspended) vs during
  recording.
- Whisper small.en model is ~500MB on disk; parakeet TDT 0.6B similarly.
  Resident memory may differ from file size.
- Document findings. If idle RSS is problematic, consider:
  - Model memory-mapping (mmap) so OS can page out under pressure.
  - Explicit model unload after extended idle, with reload on next
    activation (slower restart but lower idle footprint).

## 11.6 Daemon health and observability

- `attend narrate status` reports: daemon PID, state (idle/recording/paused),
  uptime, model loaded, idle timeout remaining.
- The pause sentinel's presence already distinguishes idle from recording.
  Combined with `record.lock` PID and process liveness check, status can
  report the full state without any new IPC.

---

## Verification

- Benchmark (11.1): log output shows model load time; compare cold vs warm.
- Integration test (11.2, 11.3): toggle on → narrate → toggle off → toggle on
  again → narrate; verify second activation is near-instant (no model reload,
  no process spawn).
- Manual test (11.3): verify `toggle` when idle resumes (doesn't spawn), and
  `toggle` when recording stops (enters idle, doesn't exit).
- Manual test (11.4): start recording, stop, wait past idle timeout → daemon
  exits cleanly, lock file removed.
- Memory test (11.5): record RSS via `ps` at idle vs recording; document.
- Status test (11.6): `attend narrate status` shows correct state transitions
  (recording → idle → recording → idle → timeout exit).
