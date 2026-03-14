# Phase 4: Unsafe Elimination & Dependency Upgrades

**Dependencies**: Phase 3 (modules reorganized, `util.rs` exists for atomic_write).
**Effort**: Medium | **Risk**: Low

---

## 4.1 Replace `libc` with `nix`

- `libc::setsid()` -> `nix::unistd::setsid()`
- `libc::kill(pid, 0)` -> `nix::sys::signal::kill(Pid::from_raw(pid), None)`
- Remove `unsafe` blocks entirely
- After 4.1 + 4.2: remove `libc` from `Cargo.toml` dependencies entirely

## 4.2 Replace manual UTC formatting with `chrono`

- `utc_now()` in `util.rs` -> `chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()`
- Remove `libc` dependency for time operations
- Eliminate all `unsafe` in `utc_now()`

## 4.3 Replace chime synthesis with `fundsp`

- Rewrite `play_chime()` and `play_flush_chime()` using fundsp oscillators and envelopes
- Cleaner, more readable, opens door for richer audio feedback later
- Add comments explaining the sound design choices

## 4.4 Replace JSONC string munching with `jsonc-parser`

- Rewrite keybinding install/uninstall in `editor/zed.rs` (or `editor/zed/` submodule if Phase 3.6 has landed)
- Parse -> edit structured AST -> serialize preserving comments
- Atomic writes for all file operations (use shared `atomic_write`)

## 4.5 Replace hand-rolled VAD downsampling

- Consider `dasp` or `rubato` (already a dep) for the 16kHz resample in `silence.rs`
- Or keep the linear interpolation with better documentation if the quality/perf tradeoff is right

## 4.6 Add `clap` color feature

- `clap = { features = ["derive", "color"] }` for colored help output

## 4.7 Atomic writes everywhere

- Audit every `fs::write()` call across the codebase
- Replace with `atomic_write()` from `util.rs`
- Skill directory installation: temp dir -> rename pattern

---

## Verification

- `grep -rn 'unsafe' src/` returns zero hits (the goal is zero unsafe in our code)
- `grep -rn 'libc::' src/` returns zero hits
- Chimes still play correctly (manual test: `attend narrate toggle`, listen for chime, `attend narrate stop`, listen for chime)
- Zed keybinding/task install/uninstall round-trips correctly (manual test: install, verify files, uninstall, verify clean)
- `grep -rn 'fs::write' src/` — each remaining hit is either in `atomic_write` itself or has an explicit justification comment
