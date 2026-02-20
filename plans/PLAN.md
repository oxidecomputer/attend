# Codebase Improvement: Status Tracker

Point Claude at this file to resume work.

## General Gates (every commit)

1. `cargo fmt --check` — clean formatting
2. `cargo clippy` — zero warnings (no new `#[allow]` without justification)
3. `cargo test` — all tests pass

## Phase Status

| Phase | Name | Status | Depends On |
|-------|------|--------|------------|
| 1 | [Foundation](./phase-01-foundation.md) | Done (003a47d) | — |
| 2 | [Type Safety & Config Simplification](./phase-02-type-safety.md) | Done (9927fbd) | Phase 1 |
| 3 | [Module Reorganization](./phase-03-module-reorg.md) | Partial (3c1acc8) | Phase 2 |
| 4 | [Unsafe Elimination & Dependency Upgrades](./phase-04-unsafe-elimination.md) | Not started | Phase 3 |
| 5 | [Error Handling Audit](./phase-05-error-handling.md) | Not started | Phase 3 |
| 6 | [Recording Daemon Improvements](./phase-06-daemon-improvements.md) | Not started | Phases 4, 5 |
| 7 | [Agent Trait Generalization](./phase-07-agent-generalization.md) | Not started | Phases 3, 5 |
| 8 | [UX Improvements](./phase-08-ux-improvements.md) | Not started | Phases 4, 6 |
| 9 | [Test Hardening](./phase-09-test-hardening.md) | Not started | Phase 3 |
| 10 | [merge.rs Deep Refactor](./phase-10-merge-refactor.md) | Not started | Phase 9 |

## Item Progress

### Phase 1: Foundation
- [x] 1.1 Add new crate dependencies
- [x] 1.2 Platform gate
- [x] 1.3 Named constants for magic numbers
- [x] 1.4 Extract all inline test modules to separate files (already done)
- [x] 1.5 Fix em-dashes and Unicode arrows in log messages
- [x] 1.6 Fix XDG comment in receive.rs
- [x] 1.7 Fix missing parent dir in hook.rs session cache write
- [x] 1.8 Audit `view/parse.rs` `parse_compact` usage (used for both stdin and CLI)

### Phase 2: Type Safety & Config Simplification
- [x] 2.1 Derive `serde::Deserialize` on `Engine` enum
- [x] 2.2 Eliminate `RawConfig`
- [x] 2.3 Add `Config::merge` method
- [x] 2.4 Camino migration (zero `to_string_lossy` / `to_str().unwrap_or_default()`)
- [ ] 2.5 Introduce `SessionId` newtype (WallClock, ModelPath dropped)

### Phase 3: Module Reorganization
- [x] 3.1 `state.rs` split — extracted `atomic_write` to `src/util.rs`
- [ ] 3.2 `json.rs` split — deferred
- [ ] 3.3 `cli/mod.rs` split — deferred
- [x] 3.4 `narrate/mod.rs` — extracted `status.rs` and `clean.rs`
- [ ] 3.5 `narrate/capture.rs` split — deferred
- [ ] 3.6 `editor/zed.rs` submodule directory — deferred
- [ ] 3.7 `merge.rs` extract `render_markdown` — deferred
- [ ] 3.8 `watch.rs` split — deferred
- [x] 3.9 `editor/mod.rs` cleanup — removed `watch_paths`/`all_watch_paths` dead code
- [x] 3.10 Future-proof editor trait — added design note on RawEditor

### Phase 4: Unsafe Elimination & Dependency Upgrades
- [ ] 4.1 Replace `libc` with `nix`
- [ ] 4.2 Replace manual UTC formatting with `chrono`
- [ ] 4.3 Replace chime synthesis with `fundsp`
- [ ] 4.4 Replace JSONC string munching with `jsonc-parser`
- [ ] 4.5 Replace hand-rolled VAD downsampling
- [ ] 4.6 Add `clap` color feature
- [ ] 4.7 Atomic writes everywhere

### Phase 5: Error Handling Audit
- [ ] 5.1 `resolve_bin_cmd` stop over-recovering
- [ ] 5.2 `receive.rs` remove legacy no-session fallback
- [ ] 5.3 `eprintln` vs `println` audit in receive.rs
- [ ] 5.4 Systematic `let _ =` audit
- [ ] 5.5 Lock file consistency
- [ ] 5.6 `auto_upgrade_hooks` rate-limit or relocate

### Phase 6: Recording Daemon Improvements
- [ ] 6.1 Reorder daemon startup
- [ ] 6.2 Remove 200ms sleep in `spawn_daemon`
- [ ] 6.3 Extract `DaemonState` struct
- [ ] 6.4 Signal handler for graceful lock cleanup
- [ ] 6.5 Add more commentary to audio and transcription logic

### Phase 7: Agent Trait Generalization
- [ ] 7.1 Refactor hook logic into generic + agent-specific
- [ ] 7.2 Split narration instructions
- [ ] 7.3 Track project-specific installations
- [ ] 7.4 Research skill format generalization

### Phase 8: UX Improvements
- [ ] 8.1 Model download during `/attend` activation
- [ ] 8.2 Auto-cleanup with configurable retention
- [ ] 8.3 Cross-platform keybindings and user-selectable keybindings
- [ ] 8.4 Elided line ranges in narration output
- [ ] 8.5 Context line tuning for highlights
- [ ] 8.6 Check parakeet-rs upstream for CTC timestamp fix
- [ ] 8.7 Narration quality: reduce cursor-only noise
- [ ] 8.8 Stop hook exit code for "no narration pending"
- [ ] 8.9 Listener restart instructions for transient failures
- [ ] 8.10 Research custom vocabulary / hotword list
- [ ] 8.11 Research: agent-driven walkthrough via Zed ACP

### Phase 9: Test Hardening
- [ ] 9.1 Test documentation pass
- [ ] 9.2 install/uninstall test coverage
- [ ] 9.3 Prop test expansion
- [ ] 9.4 Silence detector integration test

### Phase 10: merge.rs Deep Refactor
- [ ] 10.1 Comprehensive test suite for merge.rs
- [ ] 10.2 Single streaming pass rewrite
- [ ] 10.3 Documentation
