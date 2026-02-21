# Comprehensive Code Review (2026-02-20)

A systematic review of the `attend` codebase, covering maintainability,
test coverage, modularity, correctness, performance, concurrency, security,
and style. Conducted after reading every source file, config file, embedded
template, benchmark, and documentation file.

The final section cross-references the pre-plan walkthrough notes
(`WALKTHROUGH-NOTES.md`) and identifies items that remain unaddressed.

---

## 1. Architecture & Modularity

**Strengths:**

- The module hierarchy is clean and well-decomposed. Each module has a clear
  single responsibility: `editor/` reads state, `hook/` orchestrates lifecycle,
  `agent/` renders output, `narrate/` handles recording and delivery,
  `view/` handles rendering, `state/` holds core types.
- The `Agent` and `Editor` traits provide genuine extensibility. Adding a new
  editor or agent is well-documented (EXTENDING.md) and requires touching
  exactly the right files.
- The separation of `hook/` orchestration from `agent/claude/` output is
  excellent. Business logic (when to block, approve, deliver) lives in `hook/`;
  wire format (JSON keys, XML tags) lives in the agent.
- The merge pipeline (`narrate/merge.rs`) is well-factored into composable
  transformations: `collapse_cursor_only`, `union_snapshots`, `net_change_diffs`,
  each with clear input/output contracts documented in docstrings.
- `CapturedRegion` with deferred marker annotation is the right design: capture
  raw content at snapshot time, annotate at render time. Clean separation of
  concerns.

**Issues:**

- **`output.rs:26` — misplaced import.** `use crate::state::{EditorState,
  SessionId};` appears after the first function definition (`deliver_narration`).
  All imports should be at the top of the file.

- **`narrate/mod.rs` remains a light barrel module** (~106 lines) with
  `process_alive`, path helpers, `resolve_session`, and `bench`. Not a
  problem at this size, but `bench()` is arguably a CLI concern, not a
  narrate concern — it could live in `cli/narrate.rs` or a dedicated
  `benches/` helper.

## 2. Correctness

**Strengths:**

- Byte-offset resolution (`state/resolve.rs`) handles all three newline styles
  (\n, \r\n, \r) correctly, verified by a reference oracle in proptests.
- The hook decision logic (`hook/decision.rs`) is exhaustively tested across
  all 72 input combinations, with invariants verified by name.
- The `SessionMoved` ratchet (deliver once, suppress thereafter, reset on
  re-activation) is correct and well-tested in both scenario and proptest suites.

**Issues:**

- **`state/resolve.rs:293-294` — unchecked `i64` to `usize` cast.** Byte
  offsets from the Zed database are `i64`. The code casts them with `as usize`:
  ```rust
  .flat_map(|&(s, e)| [s as usize, e as usize])
  ```
  A negative offset (corrupt database row, edge case in Zed's serialization)
  would silently become a very large positive number, causing an out-of-bounds
  read on the file content. Should use `s.max(0) as usize` or
  `usize::try_from(s).unwrap_or(0)`.

- **`config.rs:63-68` — docstring belongs to wrong function.** Lines 67-68
  of `merge()`'s docstring describe `retention_duration()`:
  ```
  /// Parse `archive_retention` to a [`Duration`], returning `None` for
  /// `"forever"` (cleanup disabled). Defaults to 7 days when unset.
  ```
  This should be above `retention_duration()` at line 69, not inside
  `merge()`'s doc comment.

- **`editor/zed/db.rs:61-66` — sentinel `PathBuf::new()` for non-UTF-8 paths.**
  When a Zed DB row has a non-UTF-8 path, the code returns `PathBuf::new()`.
  This empty path passes through to `EditorState::build` where it's filtered
  out by the `Utf8PathBuf::try_from` in line 273 of `state.rs`. The behavior
  is correct, but the empty-path sentinel is fragile — if any intermediate
  code operates on paths before the UTF-8 filter, it would see `""`. Cleaner
  to skip the row entirely using `continue` or by restructuring the
  `query_map` closure to return `Ok(None)` and `filter_map` outside.

- **`view/mod.rs:384` — double unwrap with awkward fallback.**
  ```rust
  let last_rel = Line::new(lines.len()).unwrap_or(Line::new(1).unwrap());
  ```
  If `lines.len()` is 0, `Line::new(0)` returns `None`, then the fallback
  calls `Line::new(1).unwrap()`. This works but is hard to read. Prefer:
  ```rust
  let last_rel = Line::new(lines.len().max(1)).unwrap();
  ```

- **`benches/e2e.rs` — stale CLI arguments.** The benchmark invokes:
  - `["hook", "run", "claude", "user-prompt"]` — but the CLI uses
    `hook user-prompt --agent claude` (no `run` subcommand).
  - The binary with no subcommand, or with `-f json` — but `attend` requires
    a subcommand (`glance`, `look`, etc.).

  All three benchmarks would fail at runtime. This file needs updating to
  match the current CLI structure.

## 3. Test Coverage & Quality

**Strengths:**

- **Property-based testing is pervasive.** `state/tests.rs` (reorder
  properties), `view/tests.rs` (render invariants, bracket balancing, content
  preservation), `hook/tests/prop.rs` (oracle model), and
  `narrate/merge/tests/prop.rs` (merge invariants) all use proptest with
  hundreds of cases. This is exactly the right testing philosophy.

- **Hook decision testing is exemplary.** The exhaustive enumeration of all
  72 input combinations plus the random-sequence oracle model is a textbook
  approach. Each invariant test has a thorough docstring explaining the
  concurrent flow it guards against.

- **Test documentation is thorough.** Nearly every `#[test]` has a `///` doc
  comment stating the invariant. This was a walkthrough requirement and has
  been comprehensively addressed.

- **Insta snapshot tests** are used for complex rendering output
  (`view/tests.rs`, `narrate/merge/tests/render.rs`), providing regression
  protection without brittle string comparisons.

**Gaps:**

- **`audio.rs` — no unit tests.** The `resample()` function is a pure
  computation (input samples → output samples at a different rate) that could
  be tested with known signals (e.g., a single-frequency sine wave at the
  input rate should produce the same frequency at the output rate, with
  amplitude preserved within tolerance). The chime synthesis (`render_note`)
  is also testable: verify sample count matches expected duration, verify
  amplitude envelope is zero at boundaries.

- **`terminal.rs` — no tests.** `truncate_line` and `fit_to_terminal` are pure
  functions with well-defined behavior. They handle ANSI escape sequences and
  UTF-8, both of which have edge cases worth testing (multi-byte chars at
  truncation boundary, nested/malformed escapes, zero-width columns).

- **`watch.rs` — no tests.** The validation functions (`validate_options`,
  `compute_extent`, `poll_interval`) are pure and testable. The refresh logic
  is harder to test but the helper functions could be covered.

- **`narrate/status.rs` — no tests.** Status display is mostly I/O, but the
  lock-state interpretation logic (alive/stale/absent) could be extracted and
  tested, similar to how `is_lock_stale` is tested in `narrate/tests.rs`.

- **`narrate/editor_capture.rs` and `diff_capture.rs` — no tests.** The
  capture logic is threading-heavy, making it hard to unit test. However, the
  dwell threshold logic in `editor_capture` (emit only after 500ms of cursor
  stability) could be extracted into a pure state machine and tested.

- **`config.rs:70-74` — `retention_duration` parse failure is silent.** When
  `archive_retention` is set to an unparseable string (e.g., `"foo"`),
  `humantime::parse_duration` returns `Err`, and `.ok()` turns it into `None`,
  which means cleanup is *disabled* — the same as `"forever"`. A typo in the
  config silently disables cleanup. Should at minimum log a warning, or
  better, fall back to the default 7 days.

## 4. Performance

No significant performance concerns. The codebase handles data volumes
appropriate to its use case (human-speed editor interactions, seconds of
audio at a time, < 100 open files).

Minor notes:

- `union_snapshots` in `merge.rs:191-199` uses linear search (`contains`)
  for region dedup, which is O(n²). Fine for typical narrations (< 50
  regions), but a `HashSet` would be more idiomatic if regions implemented
  `Hash`.

- The Zed DB query runs a `LEFT JOIN` across three tables on every hook
  invocation. With SQLite's read-only mode and the small table sizes (active
  editors only), this is fast, but it's worth noting for editors with larger
  state stores.

## 5. Concurrency & Race Conditions

**Strengths:**

- Lock file protocol is sound: `lockfile::Lockfile` uses `O_CREAT | O_EXCL`
  for atomic creation. Stale lock detection (PID check + retry once) handles
  the common case of a killed daemon.

- Audio capture callback gracefully handles mutex poisoning with
  `if let Ok(mut guard) = chunks_ref.lock()` rather than `unwrap()`.

- Session stealing is handled correctly: atomic write of the listening file,
  marker files for ratchet state, and the `SessionRelation` enum cleanly
  classifies each hook invocation.

**Issues:**

- **`record.rs:528-533` — PID write after lock creation.** The `Lockfile::create`
  call and the `fs::write` of the PID are not atomic. If the process is
  SIGKILL'd between the two, the lock file exists but contains no PID.
  `is_lock_stale` returns `false` for non-numeric content, so the lock
  would be permanently stuck. Mitigation: the `lockfile` crate removes
  the file on `Drop`, so this only happens on `SIGKILL` (not `SIGTERM`,
  which is caught). Acceptable risk, but worth a comment explaining why.

- **`receive.rs:337-339` — `process::exit(0)` bypasses destructors.** When
  `acquire_lock_with_retry` returns `None`, the code calls
  `std::process::exit(0)`. This skips any Drop implementations in the call
  stack. In this specific case nothing critical is in scope, but the pattern
  is fragile. Consider `return Ok(())` instead.

## 6. Security

No vulnerabilities identified.

- SQL queries use rusqlite's parameterized binding (no injection).
- Subprocess spawning uses `Command::new` with explicit `.arg()` (no shell
  injection).
- File paths from the editor are filtered to `cwd` and `include_dirs` before
  delivery to the agent, preventing information leakage.
- Model downloads use HTTPS URLs to huggingface.co. No integrity verification
  (checksums) of downloaded model files — acceptable for a dev tool, but
  worth noting.

## 7. Style & Aesthetics

**Strengths:**

- Consistent use of named constants for magic numbers throughout.
- Good `tracing` integration with structured fields.
- Doc comments are thorough on public APIs and test functions.
- Error messages follow Rust convention (lowercase first letter).
- Imports are well-organized (std, external, internal groupings).

**Issues:**

- **`#[allow(dead_code)]` on `audio.rs` types.** `Recording`,
  `AudioChunk::wall_clock`, and `Recording::sample_to_offset_secs` are
  marked `dead_code`. These fields/methods were likely used during
  development but are no longer referenced. Either remove them or document
  why they exist for future use (e.g., "retained for debugging/archival").

- **`HookDecision` lacks `Clone`.** The `MockAgent` in
  `hook/tests/harness.rs:78-86` manually reconstructs decisions because
  `HookDecision` doesn't derive `Clone`. Adding `Clone` to `HookDecision`
  and `GuidanceReason` would simplify test code without any downside.

- **`Selection` lacks `Hash`.** `Position` derives `Hash` but `Selection`
  does not. Both are used as map keys (via tuple-of-references). Adding
  `Hash` to `Selection` would make it usable directly as a key and
  maintain consistency with `Position`.

- **`watch.rs:112-113,165-166` — `#[allow(clippy::too_many_arguments)]`.**
  `run_poll` and `refresh` take 9-10 parameters. A `WatchConfig` struct
  bundling `(mode, dir, interval, format, full, before, after)` would
  eliminate the lint suppression and improve readability.

## 8. Documentation

**Strengths:**

- `README.md` is comprehensive: installation, quickstart, troubleshooting,
  configuration, all commands documented with examples.
- `EXTENDING.md` is excellent: clear trait documentation, checklists, concrete
  examples, and notes for future contributors (VS Code section).
- The embedded message templates are well-written and clear.
- The narration protocol (`narration_protocol.md`) is thorough and covers
  edge cases (cursor-only narrations, receiver restart rules, stale task IDs).

**Issues:**

- **`EXTENDING.md:186` — `HookDecision` table is slightly stale.** The
  table lists `PendingNarration` as a variant, but the current code uses
  `deliver_narration()` as a separate method rather than a `HookDecision`
  variant. The `HookDecision` enum has `Silent` and `Guidance { reason,
  effect }`.

- **`benches/e2e.rs` header** references the old CLI structure. The doc
  comment is fine but the commands are wrong (see Correctness section).

---

## 9. Walkthrough Items: What Remains

Cross-referencing every item in `WALKTHROUGH-NOTES.md` against the current
codebase. Items are marked ✅ (addressed), ⚠️ (partially addressed), or
❌ (not addressed).

### Module Organization & Hierarchy
- ✅ `capture.rs` split into coordinator + `editor_capture.rs` + `diff_capture.rs`
- ✅ `narrate/mod.rs` reduced to barrel (~106 lines, acceptable)
- ✅ `cli/mod.rs` dispatch is clean match-per-variant
- ✅ `editor/zed.rs` split into submodule directory
- ✅ `json.rs` eliminated (types near consumers, utility in `util.rs`)
- ✅ `state.rs` split from utility functions
- ✅ `merge.rs` rendering separated into `render.rs`
- ✅ `EDITORS` registry visible at top of `editor/mod.rs`
- ✅ Test modules consistently extracted to `tests.rs` files
- ⚠️ `watch.rs` noted as "could be split" — terminal helpers ARE in separate
  `terminal.rs`, but `watch.rs` itself is still 280 lines with multiple
  concerns. Acceptable but not ideal.

### Config System
- ✅ `RawConfig`/`Config` collapsed; `Engine` derives `serde::Deserialize`
- ✅ `parse_engine()` removed
- ✅ Merge logic extracted to `Config::merge()`
- ✅ Config tests in separate file

### Agent Trait & Hook System
- ✅ Hook logic separated from Claude-specific code
- ✅ Agent trait provides `parse_hook_input`, output methods, install/uninstall
- ✅ Narration instructions separated: shared protocol in `narration_protocol.md`,
  Claude-specific mechanics in `skill_body.md`
- ✅ System reminder tags gated behind agent-specific output methods
- ✅ Project-specific installations tracked (`InstallMeta.project_paths`)
- ✅ `resolve_bin_cmd` correctly errors on PATH failure (no silent fallback)
- ✅ `is_attend_prompt` confirmed correct
- ✅ `auto_upgrade_hooks` effectively rate-limited (no-op after version match)
- ✅ Session IDs are a newtype (`SessionId(String)`)

### Error Handling & Over-Recovery
- ✅ `resolve_bin_cmd` fixed
- ✅ `receive.rs` no-session fallback: properly errors instead of guessing
- ✅ Stale help text updated to reference `/attend`
- ✅ Lock file approach unified (both use `lockfile` crate)
- ⚠️ `let _ =` audit: Many `let _ =` have "Best-effort" or "Intentionally
  ignored" comments, but not all. A few remain uncommented, e.g.:
  - `session_state.rs:26,44,69` — `let _ = fs::write(...)` and
    `let _ = fs::remove_file(...)` without explanation.
  - `receive.rs:139,144,150` — `let _ =` in `archive_pending` without comments
    (though the function's header comment says "Best-effort archival").
- ⚠️ `eprintln!` in `receive.rs:218`: The `None` case in
  `acquire_lock_with_retry` uses `eprintln!` which goes to stderr.
  In a background task, stderr goes nowhere. This particular case is
  when no listening session can be determined at all — arguably a
  debugging message, not an agent-facing one. Borderline.

### Unsafe Code & Dependencies
- ✅ `utc_now()` uses `chrono`
- ✅ `process_alive` uses `nix`
- ✅ `setsid()` uses `nix`
- ✅ `libc` dependency removed
- ✅ `#[cfg(not(unix))] compile_error!()` added

### Type Safety
- ✅ `SessionId(String)` newtype
- ✅ Camino adopted throughout
- ✅ Duplicate `relativize` functions consolidated (one in `state/resolve.rs`,
  one string-based in `receive.rs` — the latter is necessary because receive
  operates on event strings, not `Utf8Path` values)

### Editor Integration
- ✅ `watch_paths()` / `all_watch_paths()` dead code removed
- ✅ Cross-platform keybindings: `platform_modifier()` returns "cmd" or "super"
- ✅ JSONC crate adopted for Zed config manipulation
- ⚠️ Skill directory atomic replacement: Currently writes single SKILL.md
  atomically. The walkthrough noted stale files could linger if the skill
  grows to multiple files. Not a current issue but not preemptively addressed.

### Recording Daemon
- ✅ Startup order fixed (capture → chime → model parallel)
- ✅ 200ms sleep removed
- ✅ DaemonState struct with methods
- ✅ Magic constants named
- ✅ `setsid()` uses nix
- ✅ Unix-only compile_error added

### Audio & Transcription
- ✅ Chime synthesis uses `fundsp`
- ✅ `resample()` thoroughly documented
- ✅ Whisper parameter choices documented
- ✅ Model download surfaced during `/attend` activation

### Silence Detection
- ✅ Em-dashes replaced with colons
- ✅ Named constant for frames-per-sec
- ✅ Linear interpolation documented
- ✅ Tests in separate file

### Merge Pipeline
- ✅ Rewritten as single streaming pass
- ✅ `render_markdown` in separate `render.rs`
- ✅ Comprehensive tests (unit, snapshot, proptest)
- ✅ Elided line ranges include actual line numbers
- ✅ Context reduced to 1 line before/after (was noted as possibly excessive at 5)

### Testing
- ✅ Test documentation thorough
- ✅ Proptest preference applied extensively
- ✅ Install/uninstall JSON manipulation tested
- ✅ `parse_compact` used for both stdin and CLI, tested

### Watch & View
- ✅ Terminal helpers in separate `terminal.rs`

### Miscellaneous
- ✅ Clap `color` feature
- ✅ Auto-cleanup with `archive_retention`
- ✅ `atomic_write` as shared utility
- ✅ Cache directory creation issue resolved

---

## 10. Summary: Items Requiring Action

Ordered by severity (correctness first, then quality, then style).

### Correctness
1. **`state/resolve.rs:293-294`**: Guard against negative byte offsets from
   the editor database. Use `s.max(0) as usize` or `usize::try_from`.
2. **`benches/e2e.rs`**: Stale CLI arguments; all three benchmarks would fail.
   Update to match current subcommand structure.
3. **`config.rs:70-74`**: Unparseable `archive_retention` silently disables
   cleanup. Log a warning or fall back to default.
4. **`editor/zed/db.rs:61-66`**: Empty-path sentinel for non-UTF-8 rows.
   Skip the row instead.

### Quality
5. **`config.rs:63-68`**: Docstring for `retention_duration` is inside
   `merge()`'s docstring. Move it.
6. **`output.rs:26`**: Misplaced import after first function definition.
7. **`receive.rs:337-339`**: `process::exit(0)` bypasses destructors.
   Use `return Ok(())`.
8. **`EXTENDING.md:186`**: `HookDecision` table lists stale `PendingNarration`
   variant.

### Test Gaps
9. **`audio.rs`**: Test `resample()` with known signals.
10. **`terminal.rs`**: Test `truncate_line` and `fit_to_terminal`.
11. **`editor_capture.rs`**: Extract and test cursor dwell state machine.

### Style
12. **`audio.rs`**: Remove or document `#[allow(dead_code)]` fields.
13. **`hook/types.rs`**: Add `Clone` to `HookDecision` and `GuidanceReason`.
14. **`state/resolve.rs`**: Add `Hash` to `Selection` for consistency with
    `Position`.
15. **`watch.rs`**: Extract params into a `WatchConfig` struct.
16. **`let _ =` audit**: Add "intentionally ignored" comments to the ~6
    remaining uncommented instances.
17. **`view/mod.rs:384`**: Simplify double-unwrap pattern.

### Documentation
18. **`record.rs:528-533`**: Add comment explaining why PID write after lock
    creation is acceptable (Drop handles SIGTERM; SIGKILL is the only risk).
