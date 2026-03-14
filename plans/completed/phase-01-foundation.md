# Phase 1: Foundation

**Dependencies**: None.
**Effort**: Small | **Risk**: None

All changes are mechanical/additive. No behavioral change — the test suite is the proof.

---

## 1.1 Add new crate dependencies

Add to `Cargo.toml` (no code changes yet, just make them available):
- `nix` (replace unsafe libc calls)
- `chrono` or `time` (replace unsafe UTC formatting)
- `camino` (UTF-8 paths)
- `fundsp` (chime synthesis)
- `jsonc-parser` (Zed config manipulation)
- `indicatif` (progress bars for model download)

## 1.2 Platform gate

- Add `#[cfg(not(unix))] compile_error!("attend requires a Unix platform (macOS or Linux)")` in `main.rs`
- Note platform requirements in README

## 1.3 Named constants for magic numbers

Audit every `thread::sleep` and numeric literal across the codebase. Extract to named constants:
- `SENTINEL_POLL_INTERVAL` (50ms)
- `DAEMON_LOOP_INTERVAL` (100ms)
- `DAEMON_STARTUP_GRACE` (200ms)
- `EDITOR_POLL_INTERVAL` (100ms)
- `VAD_FRAMES_PER_SEC` (100.0, replacing `/ 100.0` in silence.rs)
- Any others found during audit

## 1.4 Extract all inline test modules to separate files

Mechanical refactor — move `#[cfg(test)] mod tests { ... }` to `tests.rs` files, replace with `#[cfg(test)] mod tests;`. Apply consistently to every module. (Already done for some; finish the rest: `config.rs`, `silence.rs`, `merge.rs`, `audio.rs`, etc.)

## 1.5 Fix em-dashes and Unicode arrows in log messages

Replace `—` and `→` in tracing messages with `:` and `->` for terminal compatibility. Audit all `tracing::debug!` / `tracing::info!` calls.

## 1.6 Fix XDG comment in receive.rs

Change hardcoded path references to "XDG cache directory" or similar.

## 1.7 Fix pre-existing `Failed to write cache` bug

The warning `Failed to write cache: No such file or directory` for `latest.json` occurs when the cache directory doesn't exist yet and the state module tries to write. Ensure parent directory exists before `atomic_write`.

## 1.8 Audit `view/parse.rs` `parse_compact` usage

Check whether `parse_compact` is used for both stdin and CLI parsing, or just one. Clarify its role and document.

---

## Verification

- No behavioral change; test suite is the proof
- Confirm no new `#[allow(unused_imports)]` for the added deps (they're used in later phases; unused deps are fine in Cargo.toml but should not be imported yet)
