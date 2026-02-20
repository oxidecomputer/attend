# Codebase Improvement Plan

Comprehensive plan from full codebase walkthrough (2026-02-19). Hard-ordered by dependency, soft-ordered by increasing risk/complexity.

## General Verification (applies to every commit)

Every commit must pass all three gates:
1. **`cargo fmt --check`** — clean formatting
2. **`cargo clippy`** — zero warnings (no new `#[allow]` without justification)
3. **`cargo test`** — all tests pass

Commit frequently at each logical unit of work. Each numbered item below (1.1, 1.2, etc.) is a natural commit boundary — some larger items may warrant multiple commits.

---

## Phase 1: Foundation
**Dependencies**: None.

### 1.1 Add new crate dependencies
Add to `Cargo.toml` (no code changes yet, just make them available):
- `nix` (replace unsafe libc calls)
- `chrono` or `time` (replace unsafe UTC formatting)
- `camino` (UTF-8 paths)
- `fundsp` (chime synthesis)
- `jsonc-parser` (Zed config manipulation)
- `indicatif` (progress bars for model download)

### 1.2 Platform gate
- Add `#[cfg(not(unix))] compile_error!("attend requires a Unix platform (macOS or Linux)")` in `main.rs`
- Note platform requirements in README

### 1.3 Named constants for magic numbers
Audit every `thread::sleep` and numeric literal across the codebase. Extract to named constants:
- `SENTINEL_POLL_INTERVAL` (50ms)
- `DAEMON_LOOP_INTERVAL` (100ms)
- `DAEMON_STARTUP_GRACE` (200ms)
- `EDITOR_POLL_INTERVAL` (100ms)
- `VAD_FRAMES_PER_SEC` (100.0, replacing `/ 100.0` in silence.rs)
- Any others found during audit

### 1.4 Extract all inline test modules to separate files
Mechanical refactor — move `#[cfg(test)] mod tests { ... }` to `tests.rs` files, replace with `#[cfg(test)] mod tests;`. Apply consistently to every module. (Already done for some; finish the rest: `config.rs`, `silence.rs`, `merge.rs`, `audio.rs`, etc.)

### 1.5 Fix em-dashes and Unicode arrows in log messages
Replace `—` and `→` in tracing messages with `:` and `->` for terminal compatibility. Audit all `tracing::debug!` / `tracing::info!` calls.

### 1.6 Fix XDG comment in receive.rs
Change hardcoded path references to "XDG cache directory" or similar.

### 1.7 Fix pre-existing `Failed to write cache` bug
The warning `Failed to write cache: No such file or directory` for `latest.json` occurs when the cache directory doesn't exist yet and the state module tries to write. Ensure parent directory exists before `atomic_write`.

### 1.8 Audit `view/parse.rs` `parse_compact` usage
Check whether `parse_compact` is used for both stdin and CLI parsing, or just one. Clarify its role and document.

**Phase 1 verification**: All changes are mechanical/additive. No behavioral change — the test suite is the proof. Confirm no new `#[allow(unused_imports)]` for the added deps (they're used in later phases; unused deps are fine in Cargo.toml but should not be imported yet).

---

## Phase 2: Type Safety & Config Simplification
**Dependencies**: Phase 1 (camino dep available, constants extracted).

### 2.1 Derive `serde::Deserialize` on `Engine` enum
Add `#[derive(serde::Deserialize)]` with `#[serde(rename_all = "lowercase")]` to `Engine`. This enables direct deserialization from TOML.

### 2.2 Eliminate `RawConfig`
With Engine deserializable, `Config` can derive `Deserialize` directly. Remove `RawConfig`, remove `parse_engine()`. Single struct.

### 2.3 Add `Config::merge` method
Extract inline merge logic from `Config::load()` into a `merge(&mut self, other: Config)` method. `load()` becomes: collect all config files → deserialize each → fold with `merge`.

### 2.4 Camino migration
Replace `PathBuf` / `Path` with `Utf8PathBuf` / `Utf8Path` throughout the codebase. Eliminates all `to_string_lossy()` and `to_str().unwrap_or_default()`. Non-UTF-8 paths fail at system boundary.
- Start with `state.rs` and `config.rs` (most path-heavy)
- Then `narrate/` modules
- Then `editor/`
- Then `view/`, `watch.rs`, `json.rs`
- Consolidate duplicate `relativize` functions (`state/resolve.rs` and `receive.rs`) into one shared utility

### 2.5 Introduce newtypes
- `SessionId(String)` — replace `Option<String>` threading
- `WallClock(String)` — ISO 8601 timestamps in AudioChunk, Recording
- `ModelPath(Utf8PathBuf)` — distinct from general file paths
- Update all function signatures, enabling the compiler to catch misuse

**Phase 2 verification**: Beyond the general gates:
- `grep -rn 'to_string_lossy' src/` returns zero hits
- `grep -rn 'to_str().unwrap_or_default()' src/` returns zero hits
- `grep -rn 'RawConfig' src/` returns zero hits
- `grep -rn 'parse_engine' src/` returns zero hits
- All existing config tests still pass (semantic equivalence with the old two-struct approach)

---

## Phase 3: Module Reorganization
**Dependencies**: Phase 2 (types settled, Config simplified — avoids reorg then re-edit).

### 3.1 `state.rs` → split and rename
- Extract `atomic_write` → `src/util.rs` (shared utility)
- Extract `cache_dir`, `listening_path`, `listening_session`, `version_path`, `installed_meta` → `src/paths.rs` or `src/cache.rs`
- Rename remaining `state.rs` → `src/editor_state.rs` (it's specifically about EditorState)

### 3.2 `json.rs` → split
- `utc_now` + `Timestamped` → replaced by chrono in Phase 4 (or move to `src/util.rs` temporarily)
- `CompactPayload` / `CompactFile` → near the CLI consumer (`src/cli/` or stay)
- `ViewPayload` / `ViewFile` → near the view consumer (`src/view/`)
- `split_selections` → `src/view/` (only used there and in json)

### 3.3 `cli/mod.rs` → split command defs from dispatch
- Command enum definitions stay in `cli/mod.rs` (or `cli/commands.rs`)
- Per-subcommand dispatch follows the `narrate.rs` pattern
- `cli/agent.rs`, `cli/view.rs`, `cli/watch.rs`, etc.
- Use `#[command(flatten)]` for single-arg variants

### 3.4 `narrate/mod.rs` → barrel module
- `bench()` → `narrate/transcribe/` (it's about transcription engines)
- `status()` → `narrate/status.rs`
- `clean()` + `clean_archive_dir()` → `narrate/clean.rs`
- What's left: path definitions, `process_alive()`, `resolve_session()`, submodule declarations

### 3.5 `narrate/capture.rs` → split
- Editor state polling thread → `narrate/editor_capture.rs` (or similar)
- File diff tracking thread → `narrate/diff_capture.rs`
- Shared `CaptureEvents` handle stays or gets its own file

### 3.6 `editor/zed.rs` → submodule directory
- `editor/zed/mod.rs` — trait impl, `Zed` struct
- `editor/zed/db.rs` — `find_db()`, database queries, `query_editors()`
- `editor/zed/keybindings.rs` — install/uninstall keybindings (JSONC manipulation)
- `editor/zed/tasks.rs` — install/uninstall tasks (JSONC manipulation)
- `editor/zed/health.rs` — `check_narration()`, `is_narration_keybinding()`

### 3.7 `merge.rs` → extract `render_markdown`
- Move `render_markdown` + `SnipConfig` to `narrate/render.rs` (presentation concern, not merging)
- `merge.rs` retains only event compression, sorting, and diff merging

### 3.8 `watch.rs` → split
- Terminal helpers (`clear_screen`, `fit_to_terminal`) → `src/terminal.rs`
- Format-specific rendering logic → cleaner match arms or helper functions
- Consider `crossterm` dependency for terminal handling

### 3.9 `editor/mod.rs` cleanup
- Remove `watch_paths()` default method (dead code, polling approach)
- Remove `all_watch_paths()` function
- Move `EDITORS` registry to top of file for visibility

### 3.10 Future-proof editor trait for line:col backends
- Note: Zed gives byte offsets; other editors may give line:col instead
- Add a comment or design note in the editor trait about a future normalization layer
- No implementation needed now, but the trait should not assume byte offsets in its contract

**Phase 3 verification**: This phase is pure reorganization — no logic changes.
- All tests pass (proves semantic equivalence)
- No file in `src/` exceeds a reasonable size (use judgment, but flag anything over ~300 lines as worth reviewing)
- `grep -rn 'pub(crate)' src/` — check that visibility didn't accidentally widen; items that were `pub(crate)` should remain so unless there's a reason
- Each commit moves code without modifying it; logic changes are deferred to later phases

---

## Phase 4: Unsafe Elimination & Dependency Upgrades
**Dependencies**: Phase 3 (modules reorganized, `util.rs` exists for atomic_write).

### 4.1 Replace `libc` with `nix`
- `libc::setsid()` → `nix::unistd::setsid()`
- `libc::kill(pid, 0)` → `nix::sys::signal::kill(Pid::from_raw(pid), None)`
- Remove `unsafe` blocks entirely
- After 4.1 + 4.2: remove `libc` from `Cargo.toml` dependencies entirely

### 4.2 Replace manual UTC formatting with `chrono`
- `utc_now()` → `chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()`
- Remove `libc` dependency for time operations
- Eliminate all `unsafe` in the former `json.rs` code

### 4.3 Replace chime synthesis with `fundsp`
- Rewrite `play_chime()` and `play_flush_chime()` using fundsp oscillators and envelopes
- Cleaner, more readable, opens door for richer audio feedback later
- Add comments explaining the sound design choices

### 4.4 Replace JSONC string munching with `jsonc-parser`
- Rewrite keybinding install/uninstall in `editor/zed/keybindings.rs`
- Rewrite task install/uninstall in `editor/zed/tasks.rs`
- Parse → edit structured AST → serialize preserving comments
- Atomic writes for all file operations (use shared `atomic_write`)

### 4.5 Replace hand-rolled VAD downsampling
- Consider `dasp` or `rubato` (already a dep) for the 16kHz resample in `silence.rs`
- Or keep the linear interpolation with better documentation if the quality/perf tradeoff is right

### 4.6 Add `clap` color feature
- `clap = { features = ["derive", "color"] }` for colored help output

### 4.7 Atomic writes everywhere
- Audit every `fs::write()` call across the codebase
- Replace with `atomic_write()` from `util.rs`
- Skill directory installation: temp dir → rename pattern

**Phase 4 verification**:
- `grep -rn 'unsafe' src/` returns zero hits (the goal is zero unsafe in our code)
- `grep -rn 'libc::' src/` returns zero hits
- Chimes still play correctly (manual test: `attend narrate toggle`, listen for chime, `attend narrate stop`, listen for chime)
- Zed keybinding/task install/uninstall round-trips correctly (manual test: install, verify files, uninstall, verify clean)
- `grep -rn 'fs::write' src/` — each remaining hit is either in `atomic_write` itself or has an explicit justification comment

---

## Phase 5: Error Handling Audit
**Dependencies**: Phase 3 (modules settled, changes are localized).

### 5.1 `resolve_bin_cmd` — stop over-recovering
- Dev mode: use `current_exe()`, done
- Release mode: `which` must succeed or return error — if we can't find the binary, neither can the agent
- Remove the fallback chain that silently papers over missing binaries

### 5.2 `receive.rs` — remove legacy no-session fallback
- The `None =>` branch that tries `narration.json` should be removed
- No session ID = error, not a guess
- Fix stale help text: `"use --session"` → reference `/attend`

### 5.3 `eprintln` vs `println` audit in receive.rs
- Agent reads stdout only; stderr goes nowhere in background tasks
- Every message intended for the agent must go to stdout
- `eprintln` reserved for debug/human-facing messages only

### 5.4 Systematic `let _ =` audit
- Review every `let _ =` across the codebase
- For each: is the error genuinely ignorable, or are we hiding a bug?
- Convert to proper error handling or add explicit `// Intentionally ignored: <reason>` comments

### 5.5 Lock file consistency
- `receive.rs` rolls its own lock with `O_CREAT | O_EXCL` while `record.rs` uses the `lockfile` crate
- Unify: either use `lockfile` everywhere, or find a PID-aware lock crate for both
- Investigate the 30-minute stale lock bug — does `lockfile::Lockfile` Drop run on SIGTERM/SIGKILL? Add signal handler if needed (`signal-hook` is already a dep)

### 5.6 `auto_upgrade_hooks` — rate-limit or relocate
- Currently runs on every hook invocation
- Consider: only on explicit user actions (`attend agent install`), or rate-limit (once per hour/session)
- At minimum, don't let upgrade failures block the hook response

**Phase 5 verification**:
- `grep -rn 'let _ =' src/` — every hit has a `// Intentionally ignored:` comment or has been converted to proper error handling
- `grep -rn 'eprintln!' src/narrate/receive.rs` — zero hits (or each is justified as human-only debug output)
- `grep -rn 'unwrap_or_default()' src/` — each hit reviewed and justified
- `grep -rn '"--session"' src/` — zero hits (stale help text removed)
- Manual test: run `attend listen` without a session → get a clear error, not a silent fallback

---

## Phase 6: Recording Daemon Improvements
**Dependencies**: Phase 4 (nix for signals), Phase 5 (error handling patterns established).

### 6.1 Reorder daemon startup
- Current: preload model → chime → start capture
- New: start capture → play chime → preload model (on thread or lazily)
- Audio accumulates while model loads; user gets immediate feedback
- Block on model readiness only when first transcription is actually needed

### 6.2 Remove 200ms sleep in `spawn_daemon`
- The daemon's lock acquisition already prevents double-spawn
- Parent returns immediately; quick double-toggle races resolve via lock

### 6.3 Extract `DaemonState` struct
- Fields: `transcriber`, `capture`, `editor_events`, `silence_detector`, `buffered_chunks`, `pre_transcribed`, `period_start`, `time_base_secs`, `sample_rate`, `session_id`
- Methods: `ingest_chunks()`, `handle_stop()`, `handle_flush()`, `transcribe_and_write()`
- Main loop becomes: `state.ingest_chunks()?; if state.check_stop()? { break; } if state.check_flush()? { continue; } sleep(POLL_INTERVAL);`
- Eliminates `#[allow(clippy::too_many_arguments)]`

### 6.4 Signal handler for graceful lock cleanup
- Use `signal-hook` (already a dep) to catch SIGTERM
- Set a flag that the daemon loop checks, same as stop sentinel
- Ensures lock file is cleaned up even if process is killed externally

### 6.5 Add more commentary to audio and transcription logic
- Document `SincInterpolationParameters` choices in `audio.rs`
- Explain the chunk/padding strategy in `resample()`
- Document Whisper parameter choices in `whisper.rs` (why greedy, why max_len=1, why token_timestamps, etc.)

**Phase 6 verification**:
- `grep -rn 'too_many_arguments' src/` returns zero hits
- Manual test: full recording lifecycle with `RUST_LOG=debug`:
  - `attend narrate toggle` → chime plays immediately (not after model load delay)
  - Speak, pause, speak → VAD log messages show correct transitions
  - `attend narrate stop` → transcription completes, narration file written
- Manual test: `attend narrate toggle`, wait 30+ seconds idle, `attend narrate toggle` again → starts cleanly (no stale lock)
- Manual test: kill daemon with `kill <pid>`, then `attend narrate toggle` → starts cleanly (signal handler cleaned up lock, or stale lock detection works)

---

## Phase 7: Agent Trait Generalization
**Dependencies**: Phase 3 (modules reorged), Phase 5 (error handling clean).

### 7.1 Refactor hook logic into generic + agent-specific
- Agent trait provides:
  - `fn parse_hook_context(&self, event: &str, stdin: &str) -> HookContext` (session_id, cwd)
  - `fn format_hook_output(&self, state: &EditorState) -> String`
  - `fn wrap_system_message(&self, content: &str) -> String`
- `hook.rs` owns shared logic: config loading, state resolution, dedup, stop-active detection
- Claude implementation: parse JSON from stdin, emit `<system-reminder>` tags

### 7.2 Split narration instructions
- Shared protocol template: what `<narration>` tags mean, listen/stop lifecycle, code/diff interleaving
- Agent-specific snippets: "Bash with `run_in_background: true`", `description: "💬"`, tool invocation patterns
- Agent trait method provides the agent-specific fragments; shared template lives in common location

### 7.3 Track project-specific installations
- When `install(project: Some(path))` is called, record the installation location
- `uninstall` without a path flag should find and clean up project-specific installs
- Prevent forgetting project-local hooks

### 7.4 Research skill format generalization
- Check if Cursor, Windsurf, or other agent harnesses have a skill/command format
- Determine what's shared vs. Claude-specific in the skill body
- Design the templating if cross-agent skills are feasible

**Phase 7 verification**:
- All existing hook tests pass unchanged (the refactor preserves behavior)
- Manual test: full `/attend` → narrate → stop flow works identically to before
- The Claude agent implementation is the only concrete impl; the trait is the new abstraction
- Adding a hypothetical second agent requires implementing only the trait methods, not touching hook.rs core logic (verify by inspection)

---

## Phase 8: UX Improvements
**Dependencies**: Phase 4 (deps available), Phase 6 (daemon restructured).

### 8.1 Model download during `/attend` activation
- When `attend listen` detects model isn't downloaded, download with progress output
- Agent sees and relays: "Downloading Parakeet model (1.2 GB)..."
- Use `indicatif` for progress bar (visible in agent's stdout capture)
- Subsequent runs skip entirely (model already present)

### 8.2 Auto-cleanup with configurable retention
- New config field: `archive_retention = "7d"` (default, e.g., 7 days; `"forever"` to disable)
- After each `archive_pending()`, prune archives older than retention
- `attend narrate clean` still exists for manual use

### 8.3 Cross-platform keybindings and user-selectable keybindings
- `cmd` is macOS-specific; should be `super` on Linux
- Check Zed's documentation for correct modifier on each platform
- Possibly: `attend editor install-keybindings --editor zed` as a separate command axis (keybinding install separate from agent install)
- Allow users to specify which keybindings to install rather than forcing defaults

### 8.4 Elided line ranges in narration output
- Change `// ... (34 lines omitted)` → `// ... (lines 45-78 omitted)`
- Makes it trivially actionable: agent can `Read` exactly those lines without arithmetic

### 8.5 Context line tuning for highlights
- Currently 5 lines before/after — evaluate whether this is excessive
- Consider making configurable or reducing default
- Measure token overhead

### 8.6 Check parakeet-rs upstream for CTC timestamp fix
- See if there's a newer release of `parakeet-rs` beyond 0.3
- Not blocking (we use TDT mode which works), but good hygiene

### 8.7 Narration quality: reduce cursor-only noise
- Add a dwell threshold: only emit cursor-only snapshots when the cursor rests at a position for >500ms (or similar). Rapid scanning generates many low-value cursor positions.
- For cursor-only events that do get emitted, include 1-2 surrounding lines of code instead of bare `// src/foo.rs:42\n❘` position. A bare position requires `attend look` to interpret, which can't be done mid-narration.
- Skip emitting the final cursor position in a narration — the stop hook already provides the latest editor context, and it's slightly more up-to-date.

### 8.8 Stop hook exit code for "no narration pending"
- The stop hook currently returns a non-zero exit code (surfaced as "blocking error") when there's no pending narration. This is noisy — it's not an error, just "nothing to deliver."
- Distinguish cleanly: exit 0 with no output for "no narration pending", non-zero only for actual errors.

### 8.9 Listener restart instructions for transient failures
- Update the skill body (`claude_skill_body.md`) to instruct: "If the listener exits without producing a narration (empty output or non-zero exit code), restart it immediately — this is a transient failure, not a permanent error."

### 8.10 Research custom vocabulary / hotword list for transcription
- Crate names (serde, rubato, camino) and domain terms are not in speech models' training data, causing transcription errors ("Certie" for serde, "Roboto" for rubato).
- Research whether Parakeet or Whisper support hotword/vocabulary biasing.
- If not natively supported, consider a post-processing step that fuzzy-matches known technical terms from the project's dependency list or a user-configurable vocabulary file.

**Phase 8 verification**:
- Manual test (8.1): delete model directory, run `/attend` in a Claude session → see download progress, then narration works
- Manual test (8.2): record several narrations, set retention to 1 second, verify old archives are pruned after next receive
- 8.4: Snapshot tests for merge/render output include line ranges in elision markers
- Manual test (8.7): narrate while rapidly scanning through a file — verify fewer cursor-only blocks than before; verify cursor-only blocks that do appear have surrounding context lines
- Manual test (8.8): stop recording with no narration pending — verify no "blocking error" in agent output
- Manual test (8.9): kill listener process, verify agent restarts it without confusion

### 8.11 Research: agent-driven walkthrough via Zed ACP
- Investigate whether Zed's ACP (Agent Control Protocol) or extension API allows an external process to:
  - Open a file in the editor
  - Navigate to a specific line/selection
  - Scroll to reveal context
- If feasible, this enables a new workflow: agent prepares a walkthrough order (by data flow, dependency graph, or risk/complexity), opens each location in sequence, and the user narrates reactions
- Flips the dynamic from "user drives, agent follows" to "agent presents, user reacts" — agent as tour guide rather than stenographer
- This is research/exploration only — no implementation commitment in this phase

---

## Phase 9: Test Hardening
**Dependencies**: Phase 3 (modules stable — don't want to document tests that are about to move).

### 9.1 Test documentation pass
- Add `///` doc comment to every `#[test]` function stating the invariant in English
- For each test, verify the body actually exercises what the name implies
- Flag any tests that are vacuously true or testing implementation details

### 9.2 install/uninstall test coverage
- Comprehensive tests for `claude.rs` JSON manipulation:
  - Empty file, existing hooks, duplicate entries, malformed JSON
  - Project-specific vs global install
  - Uninstall leaves other hooks intact
- Same for `zed/keybindings.rs` and `zed/tasks.rs` after Phase 4.4 JSONC rewrite

### 9.3 Prop test expansion
- For each unit test, ask: "Can this invariant be stated as a property over arbitrary inputs?"
- Priority targets: `merge.rs` event compression, `state/resolve.rs` offset resolution, `view/` rendering
- Expand existing prop tests in `resolve.rs` — audit for tightness (testing the right properties, not mirroring the implementation)

### 9.4 Silence detector integration test
- Synthesize audio with speech-like signal + silence gaps
- Verify split points fire at correct instants
- Verify no splits during continuous signal

**Phase 9 verification**:
- Every `#[test]` function has a `///` doc comment (enforce with a grep: `grep -B1 '#\[test\]' src/**/*.rs` and verify each has a preceding `///` line)
- Test count has increased (track before/after)
- `cargo test` still passes — new tests don't break, existing tests weren't weakened
- No test is `#[ignore]`d without a tracked reason

---

## Phase 10: merge.rs Deep Refactor
**Dependencies**: Phase 9 (comprehensive test suite in place FIRST — highest-risk change).

### 10.1 Comprehensive test suite for merge.rs
- Prop tests over arbitrary event stream permutations
- Cover every edge case: empty streams, single events, all-words, all-snapshots, interleaved, rapid-fire cursor changes
- Snapshot tests for rendered markdown output
- This must be complete before touching the implementation

### 10.2 Single streaming pass rewrite
- Replace multi-pass (`compress_snapshots` → `merge_adjacent` → `merge_diffs`) with composed fold/unfold
- Each transformation phase defined as a separate, composable function
- Document each phase's contract explicitly (input invariants → output invariants)
- Verify all existing tests still pass

### 10.3 Documentation
- Document the event stream format and each transformation's purpose
- Explain the merge semantics for diffs (net change across a period)
- Explain snapshot compression rules

**Phase 10 verification**:
- All Phase 9 merge tests pass (the entire point of writing them first)
- Snapshot test output is byte-for-byte identical to pre-refactor (no behavioral change)
- `cargo bench` (if merge-related benchmarks exist) shows no regression
- Code review: each composable function has a clear doc comment stating input→output contract
- No multi-pass iteration over the event list — single pass confirmed by inspection

---

## Summary

| Phase | Effort | Risk | Key Verification Beyond General Gates |
|-------|--------|------|---------------------------------------|
| 1. Foundation | Small | None | No behavioral change; test suite is proof |
| 2. Type safety | Medium | Low | Zero `to_string_lossy`, zero `RawConfig` |
| 3. Module reorg | Large | Low | Pure moves — no logic changes per commit |
| 4. Unsafe elimination | Medium | Low | Zero `unsafe`, zero `libc::` in src/ |
| 5. Error handling | Medium | Low-Med | Zero unjustified `let _ =`; manual error-path tests |
| 6. Daemon improvements | Medium | Medium | Manual recording lifecycle tests |
| 7. Agent generalization | Medium | Medium | Existing hook tests unchanged; trait inspection |
| 8. UX improvements | Medium | Low-Med | Manual flow tests for download, cleanup |
| 9. Test hardening | Large | None | Every test documented; count increased |
| 10. merge.rs refactor | Large | High | Byte-identical snapshot output |
