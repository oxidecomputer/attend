# Code Review Findings (2026-03-12)

Observations from narrative code review. Each item tagged with file and severity.

Reorganized for sequential triage: tests and safety nets first (red-green),
then correctness fixes, then targeted improvements bottom-up through the
dependency tree, then large reorganizations, then cross-cutting audits.

Within each section, items are ordered by dependency: foundational modules
before their consumers, and items that gate other items come first.

## Progress

Items marked with status:

- ✅ = done and merged
- 🔧 = done as part of a related fix (not a standalone item)
- ⬜ = not started

**EXTREMELY IMPORTANT:** ALWAYS update this file to reflect current status EVERY TIME you
commit changes. This is very important because it allows future agents to pick up where
you leave off.

## Topological Dependency Chart (delete from this as you complete items)

Parallelism tiers for remaining work. Items within a tier have no file
conflicts and can run as concurrent worktree agents. Tiers must be
executed sequentially (each tier depends on the prior tier being merged).

Tier 5 — Module decompositions (sequential, heavy dependencies):
  #26  merge.rs decomposition          (independent of #5)
  #31  record.rs decomposition         → requires #30 first

Tier 6 — Architecture (sequential, depends on everything above):
  #42  Extract lib.rs
  #41  narrate/ module reorganization  → after all Phase 7 decompositions

---

## Phase 1: Tests & Safety Nets

Add missing test coverage *before* touching the code those tests protect.
Red-green style: write the tests against current behavior first, then refactor
with confidence.

### ✅ 54. `src/config.rs` — No tests for `Config::merge`

**The "first wins" scalar semantics and array concatenation are untested.**
An incorrect merge order would silently use wrong config values. Add tests
covering: scalar first-wins, array concatenation order, partial layers.

**Plan:**

The `merge()` method (config.rs:136–157) implements two distinct semantics:
- **Scalars** (`engine`, `model`, `silence_duration`, `archive_retention`,
  `clipboard_capture`, `daemon_idle_timeout`): first non-`None` wins.
- **Vectors** (`include_dirs`, `ext_ignore_apps`): unconditional `extend`.

Add tests to `src/config/tests.rs`:

1. **`merge_scalar_first_wins`** — Create two configs where both set `engine`.
   Merge B into A. Assert A's value is preserved.
2. **`merge_scalar_fills_none`** — A has `engine: None`, B has `engine: Some`.
   Merge B into A. Assert A now has B's value.
3. **`merge_array_concatenation_order`** — A has `include_dirs: ["/a"]`,
   B has `include_dirs: ["/b"]`. Merge B into A. Assert A has `["/a", "/b"]`
   (A's items first).
4. **`merge_three_layers`** — Merge C into B, then B into A. Assert first-wins
   across three layers and arrays accumulate in order.
5. **`merge_empty_into_populated`** — Merge `Config::default()` into a fully
   populated config. Assert no fields change.
6. **`merge_populated_into_empty`** — Merge fully populated into default.
   Assert all scalar fields filled, arrays populated.
7. **Property test: `merge_idempotent`** — `a.merge(default) == a` for any `a`.

Files: `src/config/tests.rs`.

---

### ✅ 55. `src/state.rs` — No tests for `reorder_relative_to`

**The recency-ordering algorithm is complex (tagged sort, selection-level
reordering) but has no dedicated tests.** Add property tests covering:
new files move to front, unchanged files keep cached order, changed
selections reorder within a file.

**Plan:**

The algorithm (state.rs:414–477) does a three-phase reorder:
1. Map previous files by path → (cached_index, selections).
2. Tag each current file as "touched" (changed selections or new file) or
   "unchanged" (identical selections, gets previous index).
3. Stable partition: touched files first (in input order), then unchanged
   files (in previous cached order). Within touched files, new selections
   come before unchanged ones.

Existing property tests in `src/state/tests.rs` already cover file-level
invariants (multiset preservation, partition, idempotency, stability) and
selection-level reordering. Research confirms 11 property tests + 7
integration tests already exist.

Remaining gaps to cover:

1. **`reorder_empty_previous`** — First invocation (no cached state). Assert
   output order matches input order exactly.
2. **`reorder_empty_current`** — All files removed. Assert empty result.
3. **`reorder_touched_preserves_input_order`** — Three files all touched.
   Assert they appear in the same relative order as the input, not
   alphabetically re-sorted.
4. **`reorder_selection_dedup`** — File has duplicate selections in both
   current and previous. Assert multiset preserved (no accidental dedup).

Files: `src/state/tests.rs`.

---

### ✅ 53. `src/narrate/editor_capture.rs` — DwellTracker integration test gap

**DwellTracker unit tests cover the state machine but not its interaction
with the polling loop.** The `tick()` + `update()` interleaving in `spawn()`
has timing subtleties (tick before update on each iteration) that aren't
covered. Add an integration test that drives the full thread via
MockClock + StubEditorSource.

**Plan:**

The polling loop (editor_capture.rs:165–214) runs:
```
loop {
    sleep(100ms)
    tick(now)        // flush dwelled cursor snapshots
    state = source.current(...)
    update(files, now)  // classify as Emit / Extend / None
}
```

The DwellTracker defers cursor-only snapshots until they've dwelled for
`dwell_duration`. The integration test needs to verify this timing through
the actual thread, not just the state machine.

1. **Create `StubEditorSource`** that returns a sequence of controlled
   `EditorState` values (indexed by call count or time).
2. **Spawn the editor capture thread** with `MockClock` and the stub source.
3. **Scenario: cursor dwell fires after timeout** —
   - Stub returns cursor-only state for calls 0–5.
   - Advance MockClock past `EDITOR_POLL_MS * 5 + dwell_duration`.
   - Settle. Assert exactly one `EditorSnapshot` event emitted.
4. **Scenario: cursor replaced before dwell** —
   - Stub returns cursor-only at call 0, different cursor-only at call 1.
   - Advance past `EDITOR_POLL_MS * 2` but not past dwell.
   - Settle. Assert no events emitted (dwell timer reset).
5. **Scenario: selection interrupts cursor dwell** —
   - Stub returns cursor-only at call 0, real selection at call 1.
   - Advance past `EDITOR_POLL_MS * 2`.
   - Assert one `EditorSnapshot` with the selection (immediate emit),
     no deferred cursor snapshot.

Files: `src/narrate/editor_capture/tests.rs`.

---

### ✅ 3. `crates/mock-clock/src/tests.rs:43-50` — Misleading test

**`mock_sleep_zero_returns_immediately` doesn't test what its name claims.**
The doc comment says "returns immediately without blocking" but the assertion
only checks `clock.now() == start` (logical time didn't advance). It doesn't
verify non-blocking behavior. Either rename to reflect what it actually tests
(e.g. `mock_sleep_zero_does_not_advance_time`) or add a wall-clock timeout to
actually verify it doesn't block.

**Plan:**

The test (mock-clock/src/tests.rs:43–50):
```rust
#[test]
fn mock_sleep_zero_returns_immediately() {
    let start = Utc::now();
    let clock = MockClock::new(start);
    let sleeper = clock.for_thread();
    sleeper.sleep(Duration::ZERO);
    assert_eq!(clock.now(), start);
}
```

Two options:

**Option A (rename only):** Rename to `mock_sleep_zero_does_not_advance_time`.
Update doc comment to match. Minimal change, honest name.

**Option B (strengthen):** Keep the name, add a wall-clock assertion:
```rust
let wall_start = std::time::Instant::now();
sleeper.sleep(Duration::ZERO);
assert!(wall_start.elapsed() < std::time::Duration::from_millis(100));
assert_eq!(clock.now(), start);
```

Recommend **Option A** — wall-clock assertions are flaky under load.

Files: `crates/mock-clock/src/tests.rs`.

---

### ✅ 40. `tests/e2e.rs` — E2E test coverage gap

**The 5 e2e tests are smoke tests only.** The harness supports injection of
speech, editor state, external selections, clipboard, and time advancement
with settlement — but the tests only cover the basic happy path. Missing
coverage for: session handoff, pause/resume, flush mid-recording, yank,
idle timeout, stale lock recovery, concurrent sessions, editor state
interleaving with speech, silence-based segmentation, hook decision paths
(displaced, stolen, auto-claim). Should be a dedicated e2e test expansion
effort.

**Plan:**

The harness provides: `spawn()`, `spawn_with_stdin()`, `advance_time()`,
`advance_and_settle()`, `inject_speech()`, `wait_for_sleepers()`,
`tick_until_exit()`. Current 5 tests cover: basic narration flow, status
output, shell events, multiple speech chunks, empty collection.

Priority tests to add (ordered by coverage value):

1. **`pause_resume_continues_recording`** — Start recording, inject speech,
   pause, inject more speech (should be dropped), resume, inject speech,
   stop. Assert only speech from active periods appears.
2. **`flush_delivers_mid_recording`** — Start recording, inject speech,
   flush (should write pending without stopping), inject more speech, stop.
   Assert two narration segments delivered.
3. **`yank_copies_to_clipboard`** — Start recording, inject speech, yank.
   Assert the last segment is available via collect.
4. **`idle_timeout_stops_daemon`** — Start recording with short idle timeout
   config, inject no speech, advance time past timeout. Assert daemon exits.
5. **`editor_state_interleaved_with_speech`** — Start recording, inject
   editor snapshot, inject speech referencing file, stop. Assert narration
   contains both speech and editor context.
6. **`session_handoff`** — Start session A recording, start session B
   (should steal). Assert session A detects displacement.
7. **`stale_lock_recovery`** — Write a lock file with a non-existent PID,
   start recording. Assert lock is recovered and recording starts.
8. **`silence_based_segmentation`** — Inject speech, inject silence gap
   exceeding threshold, inject more speech. Assert two segments.

Each test follows the existing pattern: spawn CLI processes, advance mock
time, assert on collected output.

Files: `tests/e2e.rs`.

---

## Phase 2: Correctness Fixes

Bugs, safety issues, and semantic errors. Fix these before any refactoring
so the refactored code inherits correct behavior.

### ✅ 49. `src/util.rs:53-54` — `atomic_replace_dir` crash window

**Not fully atomic: crash between `remove_dir_all` and `rename` loses data.**
If the process dies after removing the old dir but before renaming staging
into place, the directory is gone with no replacement. Fix: rename old dir
to `.old` first, rename staging into place, then remove `.old`.

**Plan:**

Current code (util.rs:35–55):
```rust
let _ = fs::remove_dir_all(&staging);  // clean prior crash
fs::create_dir_all(&staging)?;
for (name, content) in files { fs::write(staging.join(name), content)?; }
let _ = fs::remove_dir_all(dir);       // ← CRASH WINDOW
fs::rename(&staging, dir)              // ← data lost if crash above
```

Replace the swap with a three-step rename:
```rust
let old = dir.with_extension("old");
let _ = fs::remove_dir_all(&old);       // clean prior crash
let _ = fs::remove_dir_all(&staging);   // clean prior crash
fs::create_dir_all(&staging)?;
for (name, content) in files { fs::write(staging.join(name), content)?; }
// Safe swap: old dir preserved until staging is in place.
if dir.exists() {
    fs::rename(dir, &old)?;             // step 1: move current → .old
}
fs::rename(&staging, dir)?;            // step 2: move staging → current
let _ = fs::remove_dir_all(&old);       // step 3: clean up .old
```

On recovery after a crash:
- If `.old` exists but `dir` doesn't → rename `.old` back to `dir`.
- If both exist → `.old` is stale, remove it.
- Add recovery logic at the top of the function.

Files: `src/util.rs`. Add tests for crash-recovery scenarios.

---

### ✅ 44. Audit all `Utc::now()` usage — Correctness

**Capture threads use `Utc::now()` directly instead of `clock.now()`.**
All timestamp sources must go through the injectable clock. Audit the
entire codebase for direct `Utc::now()` calls and replace with
`clock.now()`. The capture threads already receive a clock for sleep —
use the same clock for event timestamps. Audio capture (`AudioChunk`)
also stamps with `Utc::now()` in the cpal callback, which does not need
to be fixed because it is elided in test mode.

**Plan:**

Production code `Utc::now()` call sites:

1. **`src/cli/glance.rs:47`** — `Timestamped::at(Utc::now(), payload)`.
   Fix: thread a `Clock` parameter from the CLI entry point.
2. **`src/cli/look.rs:92`** — same pattern. Same fix.
3. **`src/narrate/audio.rs:130`** — `timestamp: Utc::now()` inside cpal
   audio callback. This runs on a cpal-owned thread with no clock access.
   Fix: document as a known limitation with a `// TODO:` comment. The
   cpal callback receives raw audio buffers on a real-time thread; passing
   a clock through the callback closure is possible but the MockClock
   would need to be `Send + Sync` (it already is). Thread a `Clock` into
   `AudioSource` and use `clock.now()` in the callback.

Steps:
1. Add `clock: Clock` parameter to `glance::run()` and `look::run()`.
2. Plumb from `main.rs` → `cli` dispatch → these functions.
3. For audio.rs, add `clock: Clock` field to `AudioSource`, use in callback.
4. Grep for any remaining `Utc::now()` in non-test code and fix.

Files: `src/cli/glance.rs`, `src/cli/look.rs`, `src/narrate/audio.rs`,
`src/cli.rs` (dispatch), `src/main.rs` (clock creation).

---

### ✅ 43. Audit `unwrap()`/`expect()` inside spawned threads — Correctness

**Panics inside spawned threads are silently swallowed.** Several places use
`unwrap()` or `expect()` inside `spawn_clock_thread` closures and
`std::thread::spawn`. A panic kills the thread but the parent may never
notice (the join handle is often stored but not checked until much later).
Audit all spawned threads for panicking unwraps and convert to proper error
propagation or at minimum log-and-continue.

**Plan:**

All spawned-thread `unwrap()` sites follow the same pattern: mutex locks
on `Arc<Mutex<Vec<Event>>>` shared state.

Affected files and locations:
1. **`src/narrate/editor_capture.rs:178,192,198,207`** —
   `events.lock().unwrap()`, `open_paths.lock().unwrap()`
2. **`src/narrate/clipboard_capture.rs:224,238`** —
   `events.lock().unwrap()`
3. **`src/narrate/diff_capture.rs:41,62,93`** —
   `open_paths.lock().unwrap()`, `events.lock().unwrap()`
4. **`src/narrate/ext_capture.rs:173,183`** —
   `events.lock().unwrap()`

Strategy: A poisoned mutex means another thread panicked while holding the
lock — the data is potentially inconsistent. For event collection, the safe
response is to log the error and stop collecting:

```rust
let Ok(mut guard) = events.lock() else {
    tracing::error!("event mutex poisoned, capture thread exiting");
    break;
};
```

Apply this pattern to every `.lock().unwrap()` in a spawned thread closure.
For `open_paths` locks, same treatment.

Additionally, check join handles: ensure the parent thread checks for panics
when joining capture threads (in `drain()` and `collect()`).

Files: `src/narrate/editor_capture.rs`, `src/narrate/clipboard_capture.rs`,
`src/narrate/diff_capture.rs`, `src/narrate/ext_capture.rs`,
`src/narrate/capture.rs` (join handle checking).

---

### ✅ 45. `process_alive()` PID reuse — Correctness

**`process_alive(pid)` is used for stale lock detection in multiple places.**
On long-running systems, PIDs can be reused. If attend's daemon exits and
another unrelated process gets the same PID, `is_lock_stale` returns false
(lock appears live). This is a narrow race but worth documenting or
mitigating (e.g. store PID + start time in lock file).

**Plan:**

Current implementation (narrate.rs:76–84): `kill(pid, None).is_ok()` —
checks if process exists and is signalable.

Usage sites:
- `src/hook/decision.rs:81`
- `src/narrate/status.rs:30,87`
- `src/narrate/record.rs:810`

Lock file format: plain text, single line with decimal PID.

**Mitigation: store PID + boot-relative start time.**

1. Change lock file format to `PID:START_NANOS\n` (e.g. `1234:98765432\n`).
2. On lock creation, write `format!("{}:{}", pid, process_start_time())`.
3. `process_start_time()`: on macOS, use `sysctl` CTL_KERN/KERN_PROC to get
   process start time. Or simpler: write `Utc::now()` timestamp and compare
   against file mtime on read (if mtime is older than a reboot, stale).
4. On stale check: parse PID + timestamp. If PID alive but timestamp doesn't
   match, treat as stale (PID was reused).

**Simpler alternative:** Store PID + a random nonce. The daemon writes
`PID:NONCE` to the lock file and also to a second "identity" file. On stale
check, verify both files agree. A reused PID won't have the same nonce.

**Simplest alternative:** Document the limitation with a comment and move on.
The race window is narrow (requires: daemon dies, PID recycled to same-user
process, attend checks lock before new process exits). On macOS with
99999-PID space, this is extremely unlikely.

Recommend: PID + start time option.

Files: `src/narrate.rs` (process_alive), lock file write sites in
`src/narrate/record.rs`.

---

### ✅ 16. `src/narrate/transcribe/{whisper,parakeet}.rs` — Incomplete checksums

**Fill in missing SHA-256 checksums for all downloadable model files.**
Whisper: only `ggml-small.en.bin` is checksummed; `ggml-base.en.bin` and
`ggml-medium.en.bin` skip verification. Parakeet: `vocab.txt` skips.
Fetch the canonical hashes from HuggingFace LFS metadata and add them.

**Plan:**

Current state:
- **Whisper** (whisper.rs:18–25): Only `ggml-small.en.bin` has a checksum.
  `ggml-base.en.bin` and `ggml-medium.en.bin` return `None`.
- **Parakeet** (parakeet.rs:42–55): `encoder-model.onnx`,
  `encoder-model.onnx.data`, `decoder_joint-model.onnx` all have checksums.
  `vocab.txt` returns `None` (small file, not LFS-tracked).

Steps:
1. Fetch SHA-256 for `ggml-base.en.bin` from HuggingFace LFS metadata:
   `curl -sL https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin | sha256sum`
   (or use the HF API: `GET /api/models/.../tree/main` for LFS pointers).
2. Same for `ggml-medium.en.bin`.
3. For Parakeet `vocab.txt`: fetch and compute SHA-256. It's small but
   should still be verified for integrity.
4. Add all checksums to their respective `expected_checksum()` match arms.

Files: `src/narrate/transcribe/whisper.rs`,
`src/narrate/transcribe/parakeet.rs`.

Also: add a command `attend model download` that preloads the model, and document it
in the installation section of the readme.

---

### ✅ 37. `src/hook/types.rs` + `src/agent/claude/input.rs:60` — Add HookKind::SessionEnd

**`SessionEnd` maps to `HookKind::SessionStart` which is confusing.**
Add a `HookKind::SessionEnd` variant (no extra fields needed) so the
mapping is honest. The current aliasing works but misleads readers.

**Plan:**

Current state:
- `HookType::SessionEnd` exists (types.rs:16).
- `HookKind` enum (types.rs:30–46) has no `SessionEnd` variant.
- Mapping in input.rs:60: `HookType::SessionEnd => HookKind::SessionStart`.

Steps:
1. Add `SessionEnd` variant to `HookKind` enum in `src/hook/types.rs`.
2. Update mapping in `src/agent/claude/input.rs:60`:
   `HookType::SessionEnd => HookKind::SessionEnd`.
3. Update any `match` on `HookKind` to handle the new variant. Search for
   `HookKind::SessionStart` matches and add `HookKind::SessionEnd` arms
   where appropriate (likely same behavior as SessionStart in most cases).
4. Update tests that construct or match `HookKind`.

Files: `src/hook/types.rs`, `src/agent/claude/input.rs`,
`src/hook.rs` (dispatch), `src/hook/tests/*.rs`.

---

## Phase 3: Dead Code Removal

Zero-risk noise reduction. Remove before refactoring so the refactored
modules don't carry dead weight.

### ✅ 11. `src/hook/session_state.rs:86-107` — Dead code

**Remove `clean_legacy_session_files()`.**
The legacy flat-file layout (`cache-*`, `displaced-*`, `moved-*`, `activated-*`
in cache root) was only used briefly. No users remain on it. Delete the function
and the call in `clean_session_markers()`.

**Plan:**

The function (session_state.rs:90–107) iterates cache root entries and
removes files matching legacy prefixes. It's called from
`clean_session_markers()` (same file).

Steps:
1. Remove the `clean_legacy_session_files()` function definition.
2. Remove the call site in `clean_session_markers()`.
3. Run `cargo test` to confirm no references remain.

Files: `src/hook/session_state.rs`.

---

### ✅ 12. `src/editor/zed/tasks.rs:7-13` — Dead code

**Remove `LEGACY_TASK_LABELS`.**
Same reasoning as observation #11: no users remain on old task label names.
Remove the legacy labels and simplify `install_task`/`uninstall_task` retain
closures.

**Plan:**

The constant (tasks.rs:7–13) lists old task label strings used for migration
cleanup. `uninstall_task()` uses it to remove both current and legacy entries.

Steps:
1. Remove the `LEGACY_TASK_LABELS` constant.
2. Simplify `uninstall_task()` to only filter on the current task label.
3. Simplify `install_task()` retain closure if it references legacy labels.
4. Update `src/editor/zed/tests.rs` if any tests reference legacy labels.

Files: `src/editor/zed/tasks.rs`, `src/editor/zed/tests.rs`.

---

### ✅ 18. `src/narrate/transcribe.rs:117-119` — Dead code

**`ensure_and_load()` is just an alias for `preload()`.**
Remove it or inline the call at the single call site.

**Plan:**

The method (transcribe.rs:117–119):
```rust
pub fn ensure_and_load(&self, path: &Utf8Path) -> anyhow::Result<Box<dyn Transcriber>> {
    self.preload(path)
}
```

Steps:
1. Find the single call site (grep for `ensure_and_load`).
2. Replace with `preload()`.
3. Remove the `ensure_and_load` method.

Files: `src/narrate/transcribe.rs`, call site file.

---

### ✅ 7. `src/config.rs:32-34` — Unnecessary config

**`ext_ignore_apps` is unnecessary.**
The default ignores Zed because it doesn't expose `AXSelectedText`, but capture
will just fail gracefully anyway — nothing goes wrong. Remove the config option
and the `default_ext_ignore_apps()` function. Simplifies both config and the
ext_capture code that consumes it.

**Plan:**

The field (config.rs:33–34) and its default (config.rs:43–45) configure which
apps to skip for macOS Accessibility text capture.

Usage sites:
- `src/narrate/capture.rs:257,293` — passed to ext_capture initialization.
- `src/narrate/record.rs:1197` — passed from config.
- `src/config/tests.rs:120,130,146` — test setup.

Steps:
1. Remove `ext_ignore_apps` field from `Config` struct.
2. Remove `default_ext_ignore_apps()` function.
3. Remove the `self.ext_ignore_apps.extend(...)` line from `merge()`.
4. Update ext_capture to not accept an ignore list (or hardcode `["Zed"]`
   internally if still desired for performance — avoid querying an app that
   will always fail).
5. Update all test code that references the field.
6. Remove from `merge()` method.

Files: `src/config.rs`, `src/narrate/capture.rs`, `src/narrate/record.rs`,
`src/narrate/ext_capture.rs`, `src/config/tests.rs`.

---

## Phase 4: Targeted Fixes — Foundational Modules

Small, focused improvements to leaf modules that the rest of the codebase
depends on. Fix these before their consumers.

---

### ✅ 9. `src/terminal.rs:61-100` — Replace hand-rolled truncation

**Replace `truncate_line` with `console::truncate_str`.**
The hand-rolled ANSI-aware truncation only handles SGR sequences (ending in `m`)
and counts chars rather than Unicode display width (misses CJK double-width).
The `console` crate handles all escape types, proper Unicode width, and edge
cases. Drop `truncate_line` and its tests in favor of the library.

**Plan:**

Current `truncate_line()` (terminal.rs:61–100) is a 40-line function that
manually parses ANSI escape bytes and counts UTF-8 char starts. Called only
by `fit_to_terminal()` (terminal.rs:105).

The `console` crate is **not** currently a dependency.

Steps:
1. Add `console = "0.15"` to `Cargo.toml` dependencies.
2. Replace `truncate_line(line, max_cols)` body with:
   ```rust
   console::truncate_str(line, max_cols - 1, "…")
   ```
   Note: verify `console::truncate_str` handles the ANSI reset (`\x1b[0m`)
   before the ellipsis, matching current behavior.
3. If behavior differs (e.g., console doesn't append reset before ellipsis),
   wrap with a thin adapter.
4. Remove the hand-rolled implementation.
5. Update tests in `src/terminal/tests.rs`: keep test cases but update
   expected output if console formats the ellipsis differently.
6. Verify CJK double-width characters are now handled correctly (add a test
   with a CJK string).

Files: `Cargo.toml`, `src/terminal.rs`, `src/terminal/tests.rs`.

---

### ✅ 8. `src/config.rs:95-130` — DRY

**`retention_duration()` and `idle_timeout()` are near-identical.**
Both parse a humantime string, handle `"forever"` → `None`, and warn+default on
error. Extract a shared `parse_optional_duration(value, field_name, default)`
helper.

**Plan:**

Both methods (config.rs:95–110 and 115–130) follow the identical pattern:
```rust
match self.FIELD.as_deref() {
    Some("forever") => None,
    Some(s) => match humantime::parse_duration(s) {
        Ok(d) => Some(d),
        Err(e) => { warn!(...); Some(DEFAULT) }
    },
    None => Some(DEFAULT),
}
```

Steps:
1. Extract a private helper:
   ```rust
   fn parse_optional_duration(
       value: Option<&str>,
       field_name: &str,
       default: Duration,
   ) -> Option<Duration> {
       match value {
           Some("forever") => None,
           Some(s) => match humantime::parse_duration(s) {
               Ok(d) => Some(d),
               Err(e) => {
                   tracing::warn!(value = s, "invalid {field_name}, using default: {e}");
                   Some(default)
               }
           },
           None => Some(default),
       }
   }
   ```
2. Rewrite `retention_duration()` and `idle_timeout()` as one-liners
   delegating to the helper.
3. Existing tests should pass unchanged.

Files: `src/config.rs`.

---

### ✅ 47. `src/config.rs` — Inconsistent duration representation

**`silence_duration` is `f64` seconds while `daemon_idle_timeout` and
`archive_retention` are humantime strings.** Use humantime for all duration
configs for consistency (e.g. `"5s"` instead of `5.0`).

**Plan:**

Current `silence_duration` (config.rs:27): `Option<f64>`.
Used in record.rs:1211 as `config.silence_duration.unwrap_or(5.0)` to get
seconds for the silence detection threshold.

Steps:
1. Change `silence_duration` field type from `Option<f64>` to
   `Option<String>`.
2. Add a `silence_duration()` method using the same
   `parse_optional_duration()` helper from item #8 — but silence is never
   `"forever"`, so use a simpler `parse_duration_with_default()` that just
   parses humantime and falls back.
3. Update the consumer in record.rs to call the method instead of
   `.unwrap_or(5.0)`. Convert `Duration` → `f64` seconds via
   `.as_secs_f64()`.
4. Update config tests.
5. Update any config file examples/docs that use `silence_duration = 5.0`
   to use `silence_duration = "5s"`.

**Note:** This is a config file format change. Existing users with
`silence_duration = 5.0` will get a parse error. Consider accepting both
formats during a transition period (try humantime first, fall back to f64
parse).

Files: `src/config.rs`, `src/narrate/record.rs`, `src/config/tests.rs`.

---

### ✅ 6. `src/state/resolve.rs:211-275` — Documentation

**`Position::from_offsets` core loop could use more inline documentation.**
The forward-scan algorithm handling `\n`, `\r\n`, `\r` with the `after_cr`
flag and buffer-chunked reading is correct but dense. A brief prose comment
explaining the state machine (especially the `after_cr` transitions) would
help future readers.

**Plan:**

The algorithm (resolve.rs:211–275) is a single-pass byte-offset to
(line, col) converter that handles three line-ending styles: `\n`, `\r\n`,
`\r` (classic Mac). The `after_cr` flag tracks whether the previous byte
was `\r`, so a following `\n` doesn't double-count.

Add comments at these points:
1. **Before the outer loop** — Prose explanation of the algorithm:
   "Single forward pass converting sorted byte offsets to (line, col)
   positions. Reads buffered chunks, tracking line/col state across
   three line-ending conventions."
2. **At the `after_cr` flag initialization** — "Tracks whether the
   previous byte was `\r`. If the next byte is `\n`, this is a `\r\n`
   pair and we don't increment line again."
3. **At the `\r` arm** — "Bare `\r` (classic Mac line ending): increment
   line, reset col. Set `after_cr` so a following `\n` is absorbed."
4. **At the `\n` arm's `after_cr` check** — "If preceded by `\r`, this
   is the `\n` half of a `\r\n` pair: clear the flag, don't increment
   line (already done for `\r`)."

Files: `src/state/resolve.rs`.

---

## Phase 5: Targeted Fixes — External Integrations

Fixes to editor, shell, browser, and hook backends. These are relatively
isolated from each other and from the narration pipeline.

### ✅ 1. `src/editor/zed/keybindings.rs:15-29` — Readability

**Deeply nested option chain in `task_already_bound` check.**
The 5-level `and_then`/`is_some_and` chain is hard to follow. Extract a
`bound_task_names(entry) -> impl Iterator<Item = &str>` helper that both
`task_already_bound` and `is_narration_keybinding` can reuse.

**Plan:**

The nested chain (keybindings.rs:15–29) drills through JSON structure:
`entry → bindings → object → values → array → [0] == "task::Spawn" && [1].task_name`.

Steps:
1. Extract a helper:
   ```rust
   fn bound_task_names(entry: &serde_json::Value) -> impl Iterator<Item = &str> {
       entry.get("bindings")
           .and_then(|b| b.as_object())
           .into_iter()
           .flat_map(|b| b.values())
           .filter_map(|v| v.as_array())
           .filter(|a| a.first().and_then(|s| s.as_str()) == Some("task::Spawn"))
           .filter_map(|a| a.get(1)?.get("task_name")?.as_str())
   }
   ```
2. Rewrite `task_already_bound` as:
   ```rust
   elements.iter().any(|e| bound_task_names(e).any(|n| n == task_name))
   ```
3. Check if `is_narration_keybinding` can also use this helper; if so,
   refactor it too.
4. Existing tests in `src/editor/zed/tests.rs` should pass unchanged.

Files: `src/editor/zed/keybindings.rs`.

---

### ✅ 14. `src/shell/{fish,zsh}.rs` — DRY

**`resolve_bin()` is duplicated identically in fish.rs and zsh.rs.**
Pull up to `shell.rs`.

**Plan:**

Both files define an identical function:
```rust
fn resolve_bin(bin_cmd: &str) -> anyhow::Result<PathBuf> {
    if std::path::Path::new(bin_cmd).is_absolute() {
        Ok(bin_cmd.into())
    } else {
        which::which(bin_cmd).map_err(|e| anyhow::anyhow!("cannot find {bin_cmd} on PATH: {e}"))
    }
}
```

Steps:
1. Move `resolve_bin()` to `src/shell.rs` as `pub(crate) fn resolve_bin(...)`.
2. Remove from both `fish.rs` and `zsh.rs`.
3. Update call sites to use `super::resolve_bin()`.

Files: `src/shell.rs`, `src/shell/fish.rs`, `src/shell/zsh.rs`.

---

### ✅ 48. `src/cli/browser_bridge.rs:100` — Unclear intent

**`fs::File::open(cache_dir())` has no comment explaining its purpose.**
Appears to be a liveness/existence check. Add a comment or remove if unused.

**Plan:**

The code (browser_bridge.rs:100):
```rust
let _ = fs::File::open(cache_dir());
```

This appears after sending a success response to the browser. The result
is discarded. Possible intents:
- **Touch the directory** to update atime (for activity tracking).
- **Verify cache dir exists** (but result is ignored).
- **Dead code** from a removed feature.

Steps:
1. Check git blame for the line to understand original intent.
2. If it's a liveness check: replace with `fs::create_dir_all(cache_dir())?`
   and add a comment: "Ensure cache directory exists for subsequent writes."
3. If it's dead code: remove it.
4. If it's an atime touch: add a comment explaining why.

Files: `src/cli/browser_bridge.rs`.

---

### ✅ 36. `src/hook.rs` — State machine documentation

**Add module-level prose documentation explaining the hook state machine.**
What are the possible session states (Active, Stolen, Inactive, Displaced)?
What transitions are legal? What does each hook type do in each state? The
logic is correct but the "why" is spread across comments on individual
branches — a unified state diagram or prose overview would help readers
orient before diving into the code.

**Plan:**

The hook system has three axes:
1. **Session relation** (`Active`, `Stolen`, `Inactive`) — whether this
   session owns the narration listener.
2. **Listen command** (`Listen`, `ListenStop`, `None`) — whether the current
   hook is an attend listen/unlisten tool call.
3. **Hook type** (`SessionStart`, `SessionEnd`, `UserPrompt`, `Stop`,
   `PreToolUse`, `PostToolUse`) — what triggered the hook.

Steps:
1. Add a module-level doc comment to `src/hook.rs` with:
   - Table: for each (session_relation, listen_command) pair, what action
     the hook takes.
   - Explanation of displacement: when session B starts listening, session
     A becomes displaced and stops receiving narration.
   - Explanation of auto-claim: when no listener exists, the first hook
     that requests narration claims the listener.
2. Keep existing inline comments on individual branches.

Files: `src/hook.rs`.

---

### ✅ 38. `src/cli/install.rs:210-241` — DRY violation in uninstall

**Uninstall rebuilds agent/editor/browser/shell lists with a repeated
conditional pattern.** Extract a helper or refactor to share the iteration
logic with install.

**Plan:**

Both install (lines 83–159) and uninstall (lines 201–260) share the
pattern: normalize input lists → look up integration by name → call trait
method → update metadata.

Uninstall additionally has the "empty means all" expansion (lines 208–241):
```rust
let agents = if uninstall_all { AGENTS.iter().map(|a| a.name()).collect() } else { agent };
let editors = if uninstall_all { EDITORS.iter()... } else { editor };
// ... repeated 4 times
```

Steps:
1. Extract a helper struct or function:
   ```rust
   fn resolve_integrations(
       agents: Vec<String>, editors: Vec<String>,
       browsers: Vec<String>, shells: Vec<String>,
       default_all: bool,
   ) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>)
   ```
   When `default_all` is true and all lists are empty, populate from the
   global `AGENTS`, `EDITORS`, `BROWSERS`, `SHELLS` constants.
2. Extract a `for_each_integration()` helper that takes a callback and
   iterates over the four lists, looking up each by name and calling the
   callback.
3. Rewrite both `install()` and `uninstall()` using these helpers.

Files: `src/cli/install.rs`.

---

### ✅ 39. `src/cli/install.rs` — Auto-detect install mode

**Add an `attend install --all` (or make it the default) that attempts every
known integration and reports results.** Instead of requiring the user to
specify `--agent claude --editor zed --shell fish`, try all backends and
report which succeeded vs which were skipped (e.g. "chrome: not detected",
"zsh: not user's shell"). Much friendlier onboarding experience.

**Plan:**

Current validation (install.rs:91–92) rejects empty lists with an error.

Steps:
1. Add `#[arg(long)] pub all: bool` to `InstallArgs`.
2. When `--all` is set (or when no specific flags given — make it the
   friendly default), iterate all known backends:
   ```rust
   for agent in AGENTS { try_install(agent); }
   for editor in EDITORS { try_install(editor); }
   // ...
   ```
3. `try_install()` catches errors and reports them as skipped:
   ```
   ✓ agent: claude
   ✓ editor: zed
   ✗ browser: chrome (not detected)
   ✗ browser: firefox (not detected)
   ✓ shell: fish
   - shell: zsh (not user's shell)
   ```
4. Return success if at least one integration installed.
5. Consider making `--all` the default when no flags are given, changing
   the current "error on empty" to "install all".

Files: `src/cli/install.rs`.

---

## Phase 6: Targeted Fixes — Narration Pipeline

Fixes within the narration subsystem, ordered by pipeline stage:
capture → transcribe → merge → render → receive.

### ✅ 21. `src/narrate/capture.rs:69-93` — Separation of concerns

**`CaptureConfig::test_mode()` should not return a `StubTranscriber`.**
The transcriber is unrelated to capture config — they're bundled only because
both need the inject router. Factor stub transcriber creation out to a separate
`test_mode` function (e.g. `test_mode::stub_transcriber()`) so capture config
construction doesn't have a surprise extra return value.

**Plan:**

Current `test_mode()` (capture.rs:69–92) returns `(Self, StubTranscriber)`.
The `StubTranscriber` is created from a global `STUB_TRANSCRIBER` OnceLock
in `test_mode.rs`.

Steps:
1. In `src/test_mode.rs`, expose a public `take_stub_transcriber()` function
   (it may already exist at line 147–150).
2. Change `CaptureConfig::test_mode()` to return only `Self`.
3. Update the caller (in `record.rs` daemon initialization) to separately
   call `test_mode::take_stub_transcriber()` for the transcriber.
4. This makes the `CaptureConfig` constructor honest about its
   responsibility.

Files: `src/narrate/capture.rs`, `src/test_mode.rs`,
`src/narrate/record.rs`.

---

### ✅ 22. `src/narrate/capture.rs` + `record.rs` — Simplify drain/collect return

**`drain()` and `collect()` should return a single `Vec<Event>`.**
The 4-tuple `(editor, diff, ext, clipboard)` is immediately concatenated into
one vec in `transcribe_and_write_to` — no per-source treatment exists. Merge
inside `drain()`/`collect()` and simplify `transcribe_and_write_to`'s signature
to take a single `Vec<Event>` for capture events (alongside browser/shell
staging which come from a different path).

**Plan:**

Current `drain()` (capture.rs:216–222) returns
`(Vec<Event>, Vec<Event>, Vec<Event>, Vec<Event>)`.

Called 3 times in record.rs (lines 255, 361, 416), destructured into 4
variables and passed separately to `transcribe_and_write_to()`.

Steps:
1. Change `drain()` to return `Vec<Event>`:
   ```rust
   pub fn drain(&self) -> Vec<Event> {
       let mut events = Vec::new();
       events.extend(self.editor.drain());
       events.extend(self.diff.drain());
       events.extend(self.ext.drain());
       events.extend(self.clipboard.drain());
       events
   }
   ```
2. Same for `collect()`.
3. Update `transcribe_and_write_to()` signature: replace four `Vec<Event>`
   parameters with one `capture_events: Vec<Event>`.
4. Update all 3 call sites in record.rs.

Files: `src/narrate/capture.rs`, `src/narrate/record.rs`.

---

### ✅ 56. `src/narrate/editor_capture.rs:165-214` — Duplicated event construction

**The `EditorSnapshot` construction is duplicated between `tick()` and `Emit`
paths (lines 177–183 vs 198–203).** Extract a helper like
`emit_snapshot(files, clock, lang_cache) -> Event`. The overall flow of the
poll loop could also benefit from more inline documentation explaining the
interplay between DwellTracker and the polling cadence.

**Plan:**

Both sites construct:
```rust
Event::EditorSnapshot {
    timestamp: now,
    last_seen: now,
    files: files.clone(),
    regions: capture_regions(...),
}
```

Steps:
1. Extract a helper function:
   ```rust
   fn make_editor_snapshot(
       files: Vec<FileEntry>,
       now: DateTime<Utc>,
       cwd: Option<&Utf8Path>,
   ) -> Event {
       Event::EditorSnapshot {
           timestamp: now,
           last_seen: now,
           files,
           regions: view::capture_regions(&files, cwd),
       }
   }
   ```
2. Replace both construction sites with calls to the helper.
3. Add inline comments to the polling loop explaining:
   - Why `tick()` runs before `update()` (flush dwelled cursors before
     processing new state).
   - What "dwell" means (cursor-only snapshots are deferred until they've
     been stable for `dwell_duration`).

Files: `src/narrate/editor_capture.rs`.

---

### ✅ 24. `src/narrate/clipboard_capture.rs:168-172` — Use blake3 for image hashing

**Replace `DefaultHasher` (SipHash) with `blake3` for clipboard image change
detection.** SipHash is designed for HashMap collision resistance, not content
fingerprinting. blake3 is faster on large inputs (SIMD), gives a proper 256-bit
hash, and is a well-known crate. Can also drop the separate `byte_len` check
since blake3 covers it.

**Plan:**

Current `hash_image_data()` (clipboard_capture.rs:164–172) uses
`std::hash::DefaultHasher` → `u64`.

Steps:
1. Add `blake3 = "1"` to `Cargo.toml`.
2. Replace function body:
   ```rust
   fn hash_image_data(img: &ImageData) -> [u8; 32] {
       blake3::hash(&img.bytes).into()
   }
   ```
3. Update the change-detection comparison: change from `u64` equality to
   `[u8; 32]` equality. The `prev_hash` field type changes from
   `Option<u64>` to `Option<[u8; 32]>`.
4. Remove the separate `byte_len` check since the hash covers content
   changes.
5. Update tests in `src/narrate/clipboard_capture/tests.rs`.

Files: `Cargo.toml`, `src/narrate/clipboard_capture.rs`,
`src/narrate/clipboard_capture/tests.rs`.

---

### ✅ 17. `src/narrate/transcribe/{whisper,parakeet}.rs` — DRY

**Shared constants and download logic are duplicated.**
`SAMPLE_RATE`, `MAX_CHUNK_SECS`, `MAX_CHUNK_SAMPLES` are identical in both
backends — move to `transcribe.rs`. The download functions (download → tmp →
checksum → rename) are structurally identical — extract a shared
`download_verified(url, dest, expected_checksum)` helper.

**Plan:**

Both backends define:
```rust
const SAMPLE_RATE: u32 = 16_000;
const MAX_CHUNK_SECS: usize = 30;
const MAX_CHUNK_SAMPLES: usize = SAMPLE_RATE as usize * MAX_CHUNK_SECS;
```

Both have download functions that:
1. Create a temp file in the model directory.
2. Download via HTTP.
3. Optionally verify SHA-256 checksum.
4. Rename temp file to final path.

Steps:
1. Move shared constants to `src/narrate/transcribe.rs`:
   ```rust
   pub(crate) const SAMPLE_RATE: u32 = 16_000;
   pub(crate) const MAX_CHUNK_SECS: usize = 30;
   pub(crate) const MAX_CHUNK_SAMPLES: usize = SAMPLE_RATE as usize * MAX_CHUNK_SECS;
   ```
2. Extract a shared download helper to `src/narrate/transcribe.rs`:
   ```rust
   pub(super) fn download_verified(
       url: &str,
       dest: &Path,
       expected_checksum: Option<&str>,
   ) -> anyhow::Result<()>
   ```
3. Update both whisper.rs and parakeet.rs to use `super::SAMPLE_RATE`,
   `super::download_verified`, etc.
4. Remove the duplicated constants and download functions from each backend.

Files: `src/narrate/transcribe.rs`, `src/narrate/transcribe/whisper.rs`,
`src/narrate/transcribe/parakeet.rs`.

---

### ✅ 51. `src/narrate/merge.rs` — Stronger typing for Event fields

**`Event::FileDiff` uses `String` for `path`** — should be `Utf8PathBuf`
for consistency. **`Event::ShellCommand` uses `String` for `cwd`** — same.
**`shell` field** could be an enum matching the `Shell` backends (`Fish`,
`Zsh`) to prevent typos.

**Plan:**

Fields to change in the `Event` enum (merge.rs:129–237):
- `FileDiff.path: String` → `Utf8PathBuf`
- `ShellCommand.cwd: String` → `Utf8PathBuf`
- `ShellCommand.shell: String` → new `ShellKind` enum

Steps:
1. Define `ShellKind` enum in `src/narrate/merge.rs` (or in `src/shell.rs`
   if it should be shared):
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq)]
   pub enum ShellKind { Fish, Zsh }
   ```
2. Change `FileDiff.path` from `String` to `Utf8PathBuf`.
3. Change `ShellCommand.cwd` from `String` to `Utf8PathBuf`.
4. Change `ShellCommand.shell` from `String` to `ShellKind`.
5. Update all construction sites (in capture modules, shell_hook, tests).
6. Update render.rs to call `.as_str()` / `.to_string()` where needed.
7. Update merge tests (may need `.into()` conversions).

Files: `src/narrate/merge.rs`, `src/narrate/render.rs`,
`src/narrate/diff_capture.rs`, `src/cli/shell_hook.rs`,
`src/narrate/merge/tests/*.rs`.

---

### ✅ 50. `src/narrate/merge.rs:363-365` — O(n²) string cloning in subsume

**`subsume_progressive_selections` clones `app`, `window_title`, `text`
strings inside the outer loop** because the inner loop borrows `events`
again. This is O(n²) string cloning. Refactor to avoid the reborrow
(e.g. pre-extract keys into a separate vec, or use indices).

**Plan:**

The function (merge.rs:345–478) has nested loops: outer iterates by index,
inner scans later events for matches. The inner loop needs shared `&events`
access while the outer has already borrowed fields from `events[i]` — hence
the cloning.

Steps:
1. Pre-extract match keys into a parallel vec:
   ```rust
   let keys: Vec<Option<(String, String, String)>> = events.iter().map(|e| {
       match e {
           Event::ExternalSelection { app, window_title, text, .. } =>
               Some((app.clone(), window_title.clone(), text.clone())),
           _ => None,
       }
   }).collect();
   ```
2. Iterate using indices and `keys[i]` instead of borrowing from `events[i]`.
3. The inner loop reads `keys[j]` (no borrow conflict) and mutates
   `events[j]` via index.
4. Total string clones: O(n) (once during key extraction) instead of O(n²).

Files: `src/narrate/merge.rs`.

---

### ✅ 52. `src/narrate/merge.rs` — O(n²) in `net_change_diffs` and `collapse_ext_selections`

**`net_change_diffs` uses `by_path.iter_mut().find(...)` linear search** —
should be `IndexMap<String, ...>` for O(1) lookup.
**`collapse_ext_selections` uses `result.iter().rposition(...)` linear
scans** for forward-merge. Both are O(n²) with many events.

**Plan:**

**`net_change_diffs` (merge.rs:561–576):**
Currently accumulates `Vec<(timestamp, path, old, new)>` and searches by
path via `.find()`.

Replace with `IndexMap<Utf8PathBuf, (timestamp, old, new)>` (preserves
insertion order for deterministic output):
```rust
let mut by_path: IndexMap<Utf8PathBuf, (DateTime<Utc>, String, String)> = IndexMap::new();
for diff in diffs {
    by_path.entry(diff.path.clone())
        .and_modify(|(_, old, new)| { *new = diff.new_content.clone(); })
        .or_insert((diff.timestamp, diff.old_content.clone(), diff.new_content.clone()));
}
```

**`collapse_ext_selections` (merge.rs:594–695):**
Currently uses `result.iter().rposition(...)` to find the last matching
entry for forward-merge.

Replace with `HashMap<(app, window_title), usize>` tracking the index of
the last entry per source:
```rust
let mut last_by_source: HashMap<(String, String), usize> = HashMap::new();
```
On each event, check `last_by_source` for O(1) merge-candidate lookup.

Steps:
1. Add `indexmap = "2"` to Cargo.toml (if not already present).
2. Rewrite `net_change_diffs` with `IndexMap`.
3. Rewrite `collapse_ext_selections` with `HashMap` index tracking.
4. All merge tests should pass unchanged (behavior preserved, only
   complexity improves).

Files: `Cargo.toml`, `src/narrate/merge.rs`.

---

### ✅ 46. `Event` debuggability

**No `Display` impl for `Event`.** Debugging merged event lists requires
`{:?}` which dumps full file contents. Add a compact `Display` impl that
shows type + timestamp + key identifiers (e.g. path, first few words).

**Plan:**

Add `impl fmt::Display for Event` in merge.rs:

```rust
impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Words { timestamp, text, .. } => {
                let preview = text.chars().take(40).collect::<String>();
                write!(f, "Words[{timestamp}]: \"{preview}…\"")
            }
            Self::EditorSnapshot { timestamp, files, .. } =>
                write!(f, "EditorSnapshot[{timestamp}]: {} files", files.len()),
            Self::FileDiff { timestamp, path, .. } =>
                write!(f, "FileDiff[{timestamp}]: {path}"),
            Self::ExternalSelection { timestamp, app, window_title, .. } =>
                write!(f, "ExtSelection[{timestamp}]: {app} ({window_title})"),
            Self::BrowserSelection { timestamp, title, url, .. } =>
                write!(f, "BrowserSelection[{timestamp}]: {title} @ {url}"),
            Self::ShellCommand { timestamp, command, .. } => {
                let preview = command.chars().take(40).collect::<String>();
                write!(f, "ShellCommand[{timestamp}]: {preview}")
            }
            Self::ClipboardSelection { timestamp, .. } =>
                write!(f, "ClipboardSelection[{timestamp}]"),
            Self::Redacted { kind, events, .. } =>
                write!(f, "Redacted({kind:?}): {} events", events.len()),
        }
    }
}
```

Use ISO-8601 compact format for timestamps (e.g. `12:34:56`).

Files: `src/narrate/merge.rs`.

---

### ✅ 27. `src/narrate/render.rs` — Write to `impl Write` instead of String

**`render_markdown` should write to an `impl Write` instead of building a
`String`.** Eliminates intermediate allocation when writing directly to a file.
Also extract the repeated block-preamble (prose termination + separator) into a
`start_block(writer, in_prose)` helper to reduce per-arm boilerplate.

**Plan:**

Current signature (render.rs:160):
```rust
pub fn render_markdown(events: &[Event], snip_cfg: SnipConfig, mode: RenderMode) -> String
```

Steps:
1. Create an internal `render_markdown_to()`:
   ```rust
   fn render_markdown_to(
       events: &[Event],
       snip_cfg: SnipConfig,
       mode: RenderMode,
       out: &mut dyn Write,
   ) -> io::Result<()>
   ```
2. Replace all `out.push_str(...)` with `write!(out, ...)?`.
3. Extract block-preamble helper:
   ```rust
   fn start_block(out: &mut dyn Write, in_prose: &mut bool) -> io::Result<()> {
       if *in_prose { writeln!(out)?; *in_prose = false; }
       writeln!(out, "---")?;
       Ok(())
   }
   ```
4. Keep the public `render_markdown()` as a convenience wrapper:
   ```rust
   pub fn render_markdown(...) -> String {
       let mut buf = Vec::new();
       render_markdown_to(events, snip_cfg, mode, &mut buf).expect("write to Vec");
       String::from_utf8(buf).expect("markdown is UTF-8")
   }
   ```

Files: `src/narrate/render.rs`.

---

### ✅ 32. `src/narrate/receive/filter.rs:114-173` — Rewrite collapse_redacted

**`collapse_redacted` uses placeholder `Event::Words` during `mem::replace`,
which is awkward.** Rewrite to `drain(..)` the input vec and process owned
values directly — no placeholders needed. Same pattern as `filter_events`
already uses on its input.

**Plan:**

Current approach: iterates by index, uses `mem::replace` with a dummy
`Event::Words` to take ownership, accumulates redacted runs, puts
non-redacted events into a result vec.

Steps:
1. Rewrite using `drain()`:
   ```rust
   fn collapse_redacted(events: &mut Vec<Event>) {
       let input: Vec<Event> = events.drain(..).collect();
       let mut result = Vec::with_capacity(input.len());
       let mut redacted_run: Vec<Event> = Vec::new();

       for event in input {
           if matches!(&event, Event::Redacted { .. }) {
               redacted_run.push(event);
           } else {
               if !redacted_run.is_empty() {
                   result.push(collapse_run(redacted_run.drain(..).collect()));
               }
               result.push(event);
           }
       }
       if !redacted_run.is_empty() {
           result.push(collapse_run(redacted_run));
       }
       *events = result;
   }
   ```
2. No placeholder events needed. Owned values flow through naturally.
3. `collapse_run()` merges a consecutive run of `Redacted` events into one.

Files: `src/narrate/receive/filter.rs`.

---

### ✅ 28. `src/view.rs` — DRY path resolution

**The abs_path resolution block (relative → absolute via cwd) is duplicated
three times** in `render_with_mode`, `render_json`, and `capture_regions`.
Extract a `resolve_abs_path(entry_path, cwd) -> Result<Utf8PathBuf>` helper.

**Plan:**

The 13-line duplicated block:
```rust
let abs_path = if entry.path.is_absolute() {
    entry.path.clone()
} else {
    let base = match cwd {
        Some(c) => c.to_path_buf(),
        None => Utf8PathBuf::try_from(std::env::current_dir()?)...,
    };
    base.join(&entry.path)
};
```

Steps:
1. Extract to a module-level function:
   ```rust
   fn resolve_abs_path(
       path: &Utf8Path,
       cwd: Option<&Utf8Path>,
   ) -> anyhow::Result<Utf8PathBuf> {
       if path.is_absolute() {
           Ok(path.to_path_buf())
       } else {
           let base = match cwd {
               Some(c) => c.to_path_buf(),
               None => Utf8PathBuf::try_from(std::env::current_dir()?)
                   .map_err(|e| anyhow::anyhow!(
                       "non-UTF-8 working directory: {}",
                       e.into_path_buf().display()
                   ))?,
           };
           Ok(base.join(path))
       }
   }
   ```
2. Replace all three call sites with `resolve_abs_path(&entry.path, cwd)?`.
3. Existing view tests should pass unchanged.

Files: `src/view.rs`.

---

### ✅ 30. `src/narrate/record.rs` — Sentinel file protocol redesign

**Replace four sentinel files with a command/status pair.**
Currently stop, flush, pause, yank each have their own sentinel file path,
but they're mutually exclusive signals. Collapse to:
1. `command` — CLI→daemon: atomic write of `stop`/`flush`/`yank`/`resume`.
   Daemon reads, acts, removes.
2. `status` — daemon→CLI: atomic write of current state
   (`recording`/`idle`/`paused`). CLI reads to decide what to do.
Separates command channel from status indication, eliminates the four-path
smell, and makes the IPC protocol clearer. Uses existing `atomic_write_str`.

**Plan:**

Current sentinel paths (record.rs:137–140):
```rust
stop_sentinel: Utf8PathBuf,
flush_sentinel: Utf8PathBuf,
pause_sentinel: Utf8PathBuf,
yank_sentinel: Utf8PathBuf,
```

Each checked via `.exists()` and acknowledged via `fs::remove_file()`.

Steps:
1. Define command and status file paths in `src/narrate.rs`:
   ```rust
   pub(crate) fn command_path() -> Utf8PathBuf { daemon_dir().join("command") }
   pub(crate) fn status_path() -> Utf8PathBuf { daemon_dir().join("status") }
   ```
2. Define command protocol:
   ```rust
   enum DaemonCommand { Stop, Flush, Pause, Resume, Yank }
   ```
   CLI writes command string via `atomic_write_str`. Daemon reads, parses,
   acts, removes.
3. Define status protocol:
   ```rust
   enum DaemonStatus { Recording, Idle, Paused }
   ```
   Daemon writes status string via `atomic_write_str` on state transitions.
   CLI reads to display or decide next action.
4. Replace the four `check_*` methods in `DaemonState` with a single
   `check_command()` that reads and matches.
5. Update CLI functions (`stop()`, `pause()`, `flush()`, `yank()`) to
   write to command_path instead of creating sentinel files.
6. Update `status.rs` to read status_path instead of checking sentinels.
7. Remove the four sentinel path functions from `src/narrate.rs`.

Files: `src/narrate.rs`, `src/narrate/record.rs`,
`src/narrate/status.rs`, `src/cli/narrate.rs`.

---

### ✅ 34. `src/narrate/status.rs` — Separate query from rendering

**`status()` mixes state querying with output formatting.** Extract a
`StatusInfo` struct (recording state, engine, model status, session, listener,
health checks, pending count, archive size, warnings) with a separate
`Display` impl or `render()` method. Enables testing the status logic
independently and supporting other output formats (e.g. JSON for agents).

**Plan:**

Current `status()` (status.rs:18–280) interleaves `fs::read_to_string`,
PID checks, and `println!` calls throughout.

Steps:
1. Define a `StatusInfo` struct:
   ```rust
   pub(crate) struct StatusInfo {
       pub recording: RecordingState,  // Active(pid) | Paused(pid) | Inactive
       pub engine: Engine,
       pub model_cached: bool,
       pub session: Option<SessionId>,
       pub listener: ListenerState,    // Active(pid) | Inactive
       pub pending_count: usize,
       pub archive_size_bytes: u64,
       pub warnings: Vec<String>,
   }
   ```
2. Extract a `pub(crate) fn query_status() -> anyhow::Result<StatusInfo>`
   that does all the filesystem reads and PID checks.
3. Implement `Display for StatusInfo` with the current formatting.
4. Rewrite `status()` as:
   ```rust
   pub(crate) fn status() -> anyhow::Result<()> {
       let info = query_status()?;
       println!("{info}");
       Ok(())
   }
   ```
5. Add a `pub(crate) fn status_json() -> anyhow::Result<serde_json::Value>`
   for agent consumption (future use).
6. Add tests for `query_status()` (mock filesystem state, assert struct
   fields).

Files: `src/narrate/status.rs`.

---

### ✅ 33. `src/narrate/receive/listen.rs` — Documentation

**Add high-level documentation connecting listen.rs to the hook/daemon
lifecycle.** The file handles lock acquisition, session handoff, model
pre-download, and polling, but it's not clear how these relate to the hook
system (PreToolUse delivering narration) and the daemon (writing pending
files). A module-level prose explanation of the full narration delivery
pipeline would help readers orient.

**Plan:**

Add a module-level doc comment covering:

1. **Where listen.rs sits in the pipeline:**
   ```
   daemon (record.rs) → writes pending files → listen.rs polls →
   filter.rs processes → delivers to agent via hook output
   ```
2. **Lock semantics:** Only one listener per session. The receive lock
   (`receive_lock_path()`) ensures exclusivity.
3. **Session handoff:** When a new session starts listening, the old
   listener detects the change via `listening_session()` mismatch and
   exits gracefully.
4. **Model pre-download:** On first listen, triggers model download so
   subsequent narrations don't block on download.
5. **Polling loop:** Sleeps `NARRATION_POLL_MS`, checks for pending files,
   processes and delivers. Exits on session steal or explicit stop.
6. **One-shot vs polling:** `wait=false` checks once and returns;
   `wait=true` enters the polling loop.

Files: `src/narrate/receive/listen.rs`.

---

## Phase 7: Module Decompositions

Large structural splits. Do these after the targeted fixes above so the
code being split is already clean. Order by dependency: split foundational
modules before their consumers.

### ✅ 5. `src/state.rs` — Organization (full decomposition)

**`state.rs` is a 519-line grab-bag. Split into focused submodules:**
1. `src/state/session_id.rs` — `SessionId` newtype + trait impls (lines 12–50)
2. `src/state/cache.rs` — cache dir resolution, `CacheDirGuard` RAII (lines 52–144)
3. `src/state/paths.rs` — `hooks_dir`, `listening_path`, `listening_session`,
   `version_path`, `InstallMeta`, `shared_cache_path`, `is_not_found` (lines 146–222)
4. `src/state/editor.rs` — `EditorState`, `FileEntry`, `build()`,
   `reorder_relative_to()` (lines 228–477)
5. `src/state/compact.rs` — `CompactPayload`, `CompactFile` JSON output (lines 484–518)

Parent `state.rs` becomes a thin re-export hub. Supersedes observation #4.

**Plan:**

Steps:
1. Create `src/state/` directory (already exists — has `resolve.rs`,
   `tests.rs`).
2. Move lines 12–50 → `src/state/session_id.rs`. Re-export:
   `pub use session_id::SessionId;`.
3. Move lines 52–144 → `src/state/cache.rs`. Contains `CACHE_DIR_OVERRIDE`,
   `set_cache_dir_override()`, `CacheDirGuard`, `ENV_CACHE_DIR`,
   `env_cache_dir()`, `cache_dir()`. Re-export all `pub` items.
4. Move lines 146–222 → `src/state/paths.rs`. Contains `hooks_dir`,
   `listening_path`, `listening_session`, `version_path`, `InstallMeta`,
   `installed_meta`, `save_install_meta`, `shared_cache_path`,
   `is_not_found`. Re-export all.
5. Move lines 228–477 → `src/state/editor.rs`. Contains `EditorState`,
   `FileEntry`, `build()`, `load_cached()`, `save_cache()`,
   `reorder_relative_to()`. Re-export.
6. Move lines 484–518 → `src/state/compact.rs`. Contains `CompactPayload`,
   `CompactFile`. Re-export.
7. `src/state.rs` becomes ~20 lines of `mod` declarations and `pub use`
   re-exports. All downstream imports are unchanged.
8. Run `cargo test` — everything should pass with no import changes.

Files: `src/state.rs` → `src/state/{session_id,cache,paths,editor,compact}.rs`.

---

### ⬜ 10. Scattered path definitions — Organization

**Centralize all cache/state path definitions into a single module.**
Path definitions are currently spread across `state.rs` (`cache_dir`,
`hooks_dir`, `listening_path`, `version_path`, `shared_cache_path`),
`hook/session_state.rs` (`sessions_dir`, `session_cache_path`,
`displaced_marker_path`, `activated_marker_path`), and likely
narrate/receive modules (staging, archive paths). Consolidate into
`src/state/paths.rs` so the full directory layout is discoverable from one
place. This subsumes the paths portion of observation #5.

**Plan:**

All path-definition functions found:

**In `src/narrate.rs` (17 functions):**
`cache_dir`, `daemon_dir`, `narration_root`, `staging_root`,
`record_lock_path`, `stop_sentinel_path`, `flush_sentinel_path`,
`pause_sentinel_path`, `receive_lock_path`, `pending_dir`,
`archive_dir`, `browser_staging_dir`, `shell_staging_dir`,
`clipboard_staging_root`, `clipboard_staging_dir`, `yanked_dir`,
`yank_sentinel_path`.

**In `src/state.rs` (5 functions):**
`cache_dir`, `hooks_dir`, `listening_path`, `version_path`,
`shared_cache_path`.

**In `src/hook/session_state.rs` (4 functions):**
`sessions_dir`, `session_cache_path`, `displaced_marker_path`,
`activated_marker_path`.

Steps:
1. After item #5 (state.rs decomposition), `src/state/paths.rs` already
   exists with the state.rs path functions.
2. Move `src/hook/session_state.rs` path functions into `src/state/paths.rs`
   (or a `src/state/paths/hook.rs` submodule).
3. Move `src/narrate.rs` path functions into `src/state/paths.rs`
   (or `src/state/paths/narrate.rs`).
4. Re-export from original modules for backward compatibility.
5. Add a module-level doc comment listing the full directory tree:
   ```
   // Cache layout:
   // ~/Library/Caches/attend/
   //   hooks/                    — hooks_dir()
   //   sessions/{id}/            — session_cache_path()
   //     displaced               — displaced_marker_path()
   //     activated               — activated_marker_path()
   //   daemon/                   — daemon_dir()
   //     lock                    — record_lock_path()
   //     command                 — command_path()
   //     status                  — status_path()
   //   narration/{id}/           — narration_root()
   //     pending/                — pending_dir()
   //     archive/                — archive_dir()
   //   staging/{id}/             — staging_root()
   //     browser/                — browser_staging_dir()
   //     shell/                  — shell_staging_dir()
   //     clipboard/              — clipboard_staging_dir()
   //   yanked/{id}/              — yanked_dir()
   ```

Files: `src/state/paths.rs`, `src/narrate.rs`, `src/hook/session_state.rs`.

---

### ⬜ 26. `src/narrate/merge.rs` — Organization (948 lines)

**Decompose merge.rs into focused submodules.**
1. `merge/event.rs` — `Event` enum, `ClipboardContent`, `RedactedKind`,
   timestamp/last_seen methods, `unified_diff`, `normalize_text`
2. `merge/subsume.rs` — `subsume_progressive_selections` (global pre-pass)
3. `merge/run.rs` — per-run transforms: `collapse_cursor_only`,
   `union_snapshots`, `net_change_diffs`, `collapse_ext_selections`,
   `dedup_browser_vs_external`, `process_run`
4. `merge/dedup.rs` — `dedup_clipboard_selections`
5. `merge.rs` — thin orchestrator: `compress_and_merge` pipeline

Also replace `SnapshotTuple` type alias with a named struct.

**Plan:**

Line ranges for each proposed submodule:
- **event.rs** (lines 109–310): `ClipboardContent`, `RedactedKind`, `Event`
  enum, `Event` impl, `unified_diff()`. ~200 lines.
- **subsume.rs** (lines 312–478): `is_cursor_only()`,
  `subsume_progressive_selections()`. ~166 lines.
- **run.rs** (lines 480–892): `SnapshotTuple`, `collapse_cursor_only`,
  `union_snapshots`, `net_change_diffs`, `collapse_ext_selections`,
  `dedup_browser_vs_external`, `normalize_text`, `dedup_clipboard_selections`,
  `process_run`. ~412 lines.
- **merge.rs** (lines 894–948): `compress_and_merge`. ~54 lines.

Steps:
1. Create `src/narrate/merge/` directory.
2. Move event types to `src/narrate/merge/event.rs`. Re-export `Event` and
   supporting types from `merge.rs`.
3. Move subsume to `src/narrate/merge/subsume.rs`.
4. Move per-run transforms to `src/narrate/merge/run.rs`.
5. Replace `type SnapshotTuple = (...)` with a named struct
   `struct Snapshot { timestamp, last_seen, files, regions }`.
6. Keep `compress_and_merge` in `src/narrate/merge.rs` as the thin
   orchestrator.
7. Move existing tests directory: already at `src/narrate/merge/tests/`.
8. Update all imports across the codebase.

Files: `src/narrate/merge.rs` → `src/narrate/merge/{event,subsume,run}.rs`.

---

### ✅ 29. `src/view.rs` — Organization

**Split view.rs into submodules.** Currently has JSON types, text/ANSI
rendering, CapturedRegion + capture, and apply_markers in 411 lines.
Split into e.g. `view/render.rs`, `view/json.rs`, `view/capture.rs`.

**Plan:**

Logical sections in view.rs:
- **JSON types** (lines 25–48): `ViewGroup`, `ViewFile`, `ViewPayload`.
- **Rendering** (lines 54–270): `Mode`, `apply_markers()`, `render()`,
  `render_with_mode()`, `render_json()`.
- **Capture** (lines 281–362): `CapturedRegion`, `capture_regions()`.

Steps:
1. Create `src/view/` directory (already exists — has `annotate.rs`,
   `detect.rs`, `parse.rs`, `tests.rs`).
2. Move JSON types to `src/view/json.rs`. Re-export from `view.rs`.
3. Move `CapturedRegion` + `capture_regions()` + `resolve_abs_path()` to
   `src/view/capture.rs`. Re-export.
4. Keep rendering functions in `src/view.rs` (they're the primary public
   surface) or move to `src/view/render.rs`.
5. `src/view.rs` becomes a thin re-export hub.

Files: `src/view.rs` → `src/view/{json,capture}.rs`.

---

### ⬜ 31. `src/narrate/record.rs` — Organization (1283 lines)

**Decompose record.rs into focused submodules:**
1. `record/daemon.rs` — `DaemonState`, main loop, command dispatch
   (simplified by command/status redesign from #30)
2. `record/transcribe.rs` — `DeferredTranscriber`, `transcribe_segment`,
   `transcribe_and_write_to`
3. `record/api.rs` — public API: `toggle`, `start`, `stop`, `pause`, `yank`,
   `flush`, `resume` + sentinel/command helpers
4. `record/spawn.rs` — `spawn_daemon` (platform-specific disclaimed spawn)
5. `record/clipboard.rs` — `copy_yanked_to_clipboard`, `collect_json_files`

The check_stop/check_flush/check_yank duplication collapses naturally with
the command/status refactor — one `match` on command content instead of
four methods with shared drain→transcribe→write patterns.

Also: remove duplicate comment at lines 1167–1168.

**Plan:**

Line ranges for proposed submodules:
- **transcriber.rs** (lines 56–112): `DeferredTranscriber`. ~57 lines.
- **daemon.rs** (lines 113–768): `DaemonState` + methods. ~656 lines.
  With command/status refactor (#30), the four check methods collapse to
  one `check_command()` with a match.
- **api.rs** (lines 769–945): `toggle`, `resume`, `is_lock_stale`, `start`,
  `flush`, `spawn_daemon`, `stop`. ~177 lines.
- **clipboard.rs** (lines 946–1052): `pause`, `yank`,
  `copy_yanked_to_clipboard`, `collect_json_files`. ~107 lines.
- **daemon_loop.rs** (lines 1126–1282): `daemon()` function. ~156 lines.

Steps:
1. **Do item #30 first** (sentinel → command/status), as it simplifies the
   daemon split.
2. Create `src/narrate/record/` directory.
3. Move each section to its submodule.
4. `src/narrate/record.rs` becomes a thin re-export hub.
5. Remove duplicate comment at lines 1167–1168.
6. Update imports across the codebase.

Files: `src/narrate/record.rs` →
`src/narrate/record/{transcriber,daemon,api,clipboard,daemon_loop}.rs`.

---

### ✅ 35. `src/narrate/{status,clean}.rs` — Placement

**`status.rs` and `clean.rs` are operational commands, not part of the
narration pipeline.** Consider moving them to a `narrate/ops/` grouping
or making them CLI-adjacent, since they don't participate in the
capture→merge→render→deliver flow.

**Plan:**

Both files are called from CLI commands (`attend narrate status`,
`attend narrate clean`). They query filesystem state but don't participate
in the recording/delivery pipeline.

Dependencies:
- **status.rs**: reads config, path functions, `process_alive()`, Engine.
- **clean.rs**: reads `narration_root()`, `clipboard_staging_root()`,
  filesystem iteration.

Options:
1. **Move to `src/narrate/ops/`**: `ops/status.rs`, `ops/clean.rs`.
   Re-export from `narrate.rs`.
2. **Move to CLI-adjacent**: `src/cli/status.rs`, `src/cli/clean.rs`.
   But they use `narrate` internals (path functions), so this creates
   a circular dependency. Better to keep in `narrate/`.

Recommend option 1: `src/narrate/ops/` grouping.

Steps:
1. Create `src/narrate/ops/` directory.
2. Move `status.rs` → `src/narrate/ops/status.rs`.
3. Move `clean.rs` → `src/narrate/ops/clean.rs`.
4. Add `pub(crate) mod ops;` to narrate.rs with re-exports.
5. Update CLI imports.

Files: `src/narrate/status.rs` → `src/narrate/ops/status.rs`,
`src/narrate/clean.rs` → `src/narrate/ops/clean.rs`.

---

## Phase 8: Architectural Decisions

Cross-cutting structural changes that reshape the module tree. Triage these
after the per-module work above, since the decompositions inform where the
boundaries should land.

### ⬜ 42. Consider extracting a `lib.rs` — Architecture

**No library crate: everything hangs off `main.rs`.** This makes mid-level
integration tests (calling internal APIs without spawning subprocesses)
impossible. A `lib.rs` with a carefully designed public surface would enable
richer testing and potential reuse. Requires careful thought about what to
expose.

**Plan:**

Current `main.rs` (48 lines): 15 `mod` declarations + `fn main()`.

Steps:
1. Create `src/lib.rs` with all `mod` declarations moved from `main.rs`.
2. Keep `main.rs` minimal:
   ```rust
   fn main() -> anyhow::Result<()> {
       attend::cli::Cli::parse().run()
   }
   ```
3. Decide on public surface: initially, make everything `pub(crate)` — the
   lib.rs just enables integration tests, not external consumption.
4. Move test_mode initialization from main.rs into the CLI entry point.
5. Integration tests in `tests/` can now `use attend::narrate::merge::*`
   etc. without spawning subprocesses.

**Risk:** Adding `lib.rs` changes Cargo's compilation model (binary + library
crate). May affect compile times. Test with `cargo build --timings`.

Files: `src/main.rs`, new `src/lib.rs`.

---

### ⬜ 41. `narrate/` module is overloaded — Architecture

**The `narrate/` subtree is the largest module and contains too many
responsibilities:** daemon, capture, merge, render, receive, status, clean,
chime, audio, silence, transcription. Many individual refactors above
(record decomposition #31, merge decomposition #26, integrations
consolidation #13, status/clean placement #35) point at the same root
issue. Needs a higher-level reorganization — possibly splitting into
`pipeline/` (capture→merge→render), `daemon/` (record, sentinels, IPC),
and `delivery/` (receive, listen, pending).

**Plan:**

After the individual decompositions in Phase 7, `narrate/` will already be
cleaner. The remaining question is whether the top-level grouping makes
sense.

Current file count: 15 files, ~5,938 lines.

Proposed reorganization:
```
src/narrate/
  audio.rs            — microphone capture, resampling
  silence.rs          — VAD-based silence detection
  chime.rs            — audio feedback tones
  transcribe/         — Whisper/Parakeet backends
  capture/            — coordinator + 4 capture threads
  merge/              — event merge & compression (already decomposed)
  render.rs           — markdown rendering
  record/             — recording daemon (already decomposed)
  receive/            — delivery pipeline (already exists)
  ops/                — status, clean (already moved in #35)
```

This is mostly already achieved by items #26, #31, #35. The remaining
question is whether to pull audio/silence/chime/transcribe into a separate
`audio/` subtree. Recommend deferring — the current flat grouping under
`narrate/` is fine once the large files are decomposed.

Files: depends on earlier items.

---

### ⬜ 56. Flaky e2e yank tests — Bug

**`yank_writes_to_yanked_dir` and `yank_while_paused_delivers_content` fail
intermittently under parallel test load.** The failure is an empty archive
directory: the daemon exits but the yanked/archived narration files are
missing. Passes reliably in isolation; only fails when run alongside the
full suite.

**Root cause investigation so far:**
- The daemon is a grandchild process (forked by the CLI), not in the
  harness's `self.children`. Its exit is detected via socket EOF in
  `advance_time`, not `try_wait`.
- The bare `assert!(!h.has_daemon())` after `tick_until_exit(yank)` was
  wrong because `daemon_pid` is only cleared by `advance_time` (socket
  write/read), not by `has_daemon()`. Fixed to use `tick_until_daemon_exits`.
- But the *content* assertion still fails intermittently: the archive dir
  is empty. The daemon finalized and exited, but the files aren't there.
  This suggests a race in the yank CLI's `copy_yanked_to_clipboard` or
  in the daemon's `finalize_and_write`.

**Reproduces on `main` (pre-existing).** Not introduced by #51.

**Plan:** Instrument the yank flow to determine whether the daemon fails
to write yanked files, or the CLI fails to read/archive them. Add a
deterministic wait or synchronization point.

Files: `tests/e2e.rs`, `src/narrate/record.rs`, `crates/test-harness/src/lib.rs`.

## Phase 9: Untriaged

Items discovered after the initial review. Triage and slot into the
appropriate phase/tier before implementation.

### ✅ 57. Hook installation not idempotent — duplicates in settings

**User-reported: 13 duplicate hook entries per hook type in
`~/.claude/settings.json`, all without `_installed_by` marker.**

**Root cause confirmed:** Claude Code strips unknown JSON fields from
`settings.json` on session start. The `_installed_by: "attend"` marker
is written by `attend install` but removed by Claude Code before the
next `is_our_hook()` check runs. Every auto-upgrade therefore adds a
new entry without cleaning up the (now marker-less) previous ones.

**Fix applied:** Added a command-pattern fallback to `is_our_hook()`.
If no `_installed_by` marker is found, check whether any hook command
starts with `"attend "`. This correctly identifies both marked and
stripped entries. The marker is retained as defense-in-depth for
future Claude Code versions that might preserve extra fields.

After the fix, `attend install --agent claude` deduplicates: 13 → 1
per hook type.

Files: `src/agent/claude/settings.rs`, `src/agent/claude/settings/tests.rs`.
