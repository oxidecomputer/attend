# Phase 14: Pause

**Dependencies**: Phase 6 (daemon restructured).
**Effort**: Medium | **Risk**: Low

---

## Motivation

Today, narration is all-or-nothing: recording or stopped. A user who takes a
phone call, has a side conversation, or steps away must either stop narration
(losing the "still recording" context) or let irrelevant audio pollute the
transcript. Pause mutes all capture channels without delivering content to the
agent, so the user can resume seamlessly.

**This is also a deliberate prelude to phase 18 (persistent daemon).** Once
pause fully suspends all capture with zero resource usage, the daemon's idle
state is simply "paused." Stop becomes flush+pause rather than flush+exit, and
the daemon stays resident for instant restart. Phase 18's scope shrinks to
session lifecycle management and IPC, with the heavy lifting (suspend/resume
all capture) already done here.

---

## Design

### Sentinel

`~/.cache/attend/pause` (same pattern as stop/flush).

The pause sentinel doubles as state: **exists = paused, absent = not paused**.
The CLI `attend narrate pause` writes the file if absent (pause) or deletes it
if present (resume). The daemon checks each loop iteration: on first detection
of the file, transition to paused; on detection that it's gone, transition to
resumed. No second sentinel needed. This also means `narrate status` can
report "paused" just by checking for the file.

### Daemon behavior on pause (file appears)

1. Play a distinct pause chime.
2. Flush accumulated content (same as `check_flush()`): drain audio, drain
   editor/ext events, transcribe, merge, and write the pending narration file.
   Everything captured up to this moment is real content and must be preserved.
3. Reset the silence detector to Idle.
4. Set `paused = true` on `DaemonState`.
5. Record `pause_started_at: DateTime<Utc>` for time accounting on resume.

### Full capture suspension

While paused, all capture is fully suspended — effectively zero resource usage:

- The cpal audio stream is paused via `audio::CaptureHandle::pause()` (new
  method, delegates to `cpal::Stream::pause()`). This stops the audio
  callback, saves CPU/power, and on macOS turns off the mic indicator dot — a
  visible signal that pause is active. When loopback capture (phase 17) lands,
  both streams are paused.
- The editor/diff/ext capture threads are paused via a shared `paused`
  `AtomicBool` on the `capture::CaptureHandle`. All three threads already
  share an `AtomicBool` stop flag and poll in a `while !stop.load() { sleep;
  poll; }` loop. Adding a paused check is minimal: when paused, the thread
  sleeps at longer intervals (e.g. 500ms) and skips polling entirely.
- The daemon main loop itself continues polling sentinels at
  `DAEMON_LOOP_POLL_MS` (100ms) — this is necessary so stop/yank/resume are
  detected promptly.

### Daemon behavior on resume (file disappears while `paused == true`)

1. Resume the cpal audio stream via `audio::CaptureHandle::resume()`.
2. Unpause the editor/diff/ext capture threads (clear the `paused` flag).
3. Play a distinct resume chime.
4. Set `paused = false` on `DaemonState`.
5. Reset period timestamps (`period_start`, `period_start_utc`, `last_drain`)
   so the next segment's word timestamps are accurate.
6. Increment `time_base_secs` by the wall-clock duration since
   `pause_started_at` (computed as `Utc::now() - pause_started_at`).
7. Clear `pre_transcribed` (stale context from before pause).

### Edge cases

**Stop/yank while paused**: Works normally. `check_stop()` or `check_yank()`
fires, but there's nothing new to transcribe (audio was discarded during
pause). Editor/ext capture handles are still `Some` so `.collect()` works.
Note that pre-pause content may already exist in `pending/` from the pause
flush.

**Flush while paused**: Handled but produces no new output (buffers are empty
since pause). `check_flush()` deletes the sentinel, resets state as usual.

---

## Chimes

| Action | Chime | Description |
|--------|-------|-------------|
| Pause  | Single D5 (~80ms), low amplitude | Soft "tap": signals suspension |
| Resume | D5→E5 ascending pair (~80ms each) | Rising pair: signals continuation |
| Empty  | Single A4 (~80ms), low amplitude | Low tone: nothing was captured |

The empty chime plays when stop/yank/flush produces no content (nothing to
transcribe, no editor events, no selections). This alerts the user if their
mic was muted or they forgot to speak. Applies regardless of pause state:
pause flushes pre-pause content to pending, so stop-while-paused can still
have output (from before the pause).

---

## Keybinding

| Action | macOS | Linux |
|--------|-------|-------|
| **Pause / resume** | `cmd-{` | `super-{` |

New Zed task: `"attend: pause narration"` → `attend narrate pause`

Uses `"hide": "always"` and `"reveal": "never"` (same as toggle/start).

---

## Task breakdown

| # | Task | Depends On | Files |
|---|------|------------|-------|
| 1 | Pause sentinel path + `attend narrate pause` CLI | — | `narrate.rs`, `cli/narrate.rs` |
| 2 | `audio::CaptureHandle::pause()` / `resume()` | — | `narrate/audio.rs` |
| 3 | `capture::CaptureHandle` paused flag + `pause()` / `resume()` | — | `narrate/capture.rs` |
| 4 | Editor/diff/ext threads: check `paused` flag, skip polling when set | 3 | `narrate/editor_capture.rs`, `narrate/diff_capture.rs`, `narrate/ext_capture.rs` |
| 5 | `DaemonState` pause support (flush-then-suspend, resume detection) | 1, 2, 3 | `narrate/record.rs` |
| 6 | Pause/resume chimes + empty chime | — | `narrate/audio.rs` |
| 7 | Wire chimes into daemon (pause, resume, empty on stop/flush) | 5, 6 | `narrate/record.rs` |
| 8 | `narrate status`: report "paused" state | 1 | `narrate/status.rs` |
| 9 | Zed task + keybinding for pause | — | `editor/zed.rs`, `editor/zed/keybindings.rs` |
| 10 | Tests: pause/resume sentinel round-trip, full suspend, empty chime | All | tests |

---

## Verification

- Start recording, speak, press pause hotkey → hear pause chime, mic indicator
  turns off, pre-pause narration appears in pending.
- Press pause hotkey again → hear resume chime, mic indicator turns on.
- Speak more, stop → both pre-pause and post-resume content are delivered.
- Start recording, pause, stop → empty chime, no content delivered.
- `attend narrate status` reports "paused" while paused, "recording" otherwise.
