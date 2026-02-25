# Codebase Improvement: Status Tracker

Claude: Please update this file every time you complete a task to ensure that
you keep track of your work.

Do not skip tasks. These were carefully planned already; if there's a good
reason why one cannot be performed, feel free to call it out, but if a task can
be performed as specified, please do it.

## General Gates (every commit)

1. `cargo fmt` — clean formatting automatically (no need to check it first)
2. `cargo clippy` — zero warnings (no new `#[allow]` without approval from user)
3. `cargo test` — all tests pass

## Phase Status

| Phase | Name | Status | Depends On |
|-------|------|--------|------------|
| 1 | [Foundation](./phase-01-foundation.md) | Done (003a47d) | — |
| 2 | [Type Safety & Config Simplification](./phase-02-type-safety.md) | Done (6990ff3) | Phase 1 |
| 3 | [Module Reorganization](./phase-03-module-reorg.md) | Done (6733577) | Phase 2 |
| 4 | [Unsafe Elimination & Dependency Upgrades](./phase-04-unsafe-elimination.md) | Done (2393beb) | Phase 3 |
| 5 | [Error Handling Audit](./phase-05-error-handling.md) | Done (066144d) | Phase 3 |
| 6 | [Recording Daemon Improvements](./phase-06-daemon-improvements.md) | Done | Phases 4, 5 |
| 7 | [Agent Trait Generalization](./phase-07-agent-generalization.md) | Done (35571a3) | Phases 3, 5 |
| 8 | [UX Improvements](./phase-08-ux-improvements.md) | Done (774ecae) | Phases 4, 6 |
| 9 | [Test Hardening](./phase-09-test-hardening.md) | Done | Phase 3 |
| 10 | [merge.rs Deep Refactor](./phase-10-merge-refactor.md) | Done | Phase 9 |
| 12a | [External Context Sources (Part A)](./phase-12-context-sources.md) | Done | Phase 10 |
| 12b | [Firefox Native Messaging (Part B)](./phase-12-context-sources.md) | Done | Phase 12a |
| 13 | [No-Session Support](./phase-13-no-session.md) | Done (0bea28a) | Phase 12b |
| 14 | [Pause](./phase-14-pause.md) | Done (9729a96) | Phase 6 |
| 15 | [Shell Hook Integration](./phase-15-shell-hooks.md) | Done (4bfd7aa) | Phases 6, 10, 12b, 13 |
| 16 | [Yank-to-Clipboard](./phase-16-yank.md) | Done | Phases 13, 14 |
| 17 | [Loopback Audio Capture](./phase-17-loopback-capture.md) | Not started | Phases 6, 10 |
| 18 | [Persistent Daemon](./phase-18-persistent-daemon.md) | Not started | Phase 14 |

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
- [x] 2.5 Introduce `SessionId` newtype (WallClock, ModelPath dropped)

### Phase 3: Module Reorganization
- [x] 3.1 `state.rs` split — extracted `atomic_write` to `src/util.rs`
- [x] 3.2 `json.rs` split — completed (1e31c01, 5acf9c3)
- [x] 3.3 `cli/mod.rs` split — extracted glance, look, install handlers (512ad49)
- [x] 3.4 `narrate/mod.rs` — extracted `status.rs` and `clean.rs`
- [x] 3.5 `narrate/capture.rs` split — editor_capture.rs + diff_capture.rs (6733577)
- [x] 3.6 `editor/zed.rs` submodule directory — split into db, jsonc, keybindings, tasks, health (7a3c126)
- [x] 3.7 `merge.rs` extract `render_markdown` — split into narrate/render.rs (cc1e2ae)
- [x] 3.8 `watch.rs` split — extracted terminal helpers to src/terminal.rs (ee29bdd)
- [x] 3.9 `editor/mod.rs` cleanup — removed `watch_paths`/`all_watch_paths` dead code
- [x] 3.10 Future-proof editor trait — added design note on RawEditor

### Phase 4: Unsafe Elimination & Dependency Upgrades
- [x] 4.1 Replace `libc` with `nix` + `crossterm` (a2ead64)
- [x] 4.2 Replace manual UTC formatting with `chrono` (a2ead64)
- [x] 4.3 Replace chime synthesis with `fundsp` (6d323a8)
- [x] 4.4 Replace JSONC string munching with `jsonc-parser` (43025d5)
- [x] 4.5 VAD downsampling: documented as intentional (linear for VAD is sufficient)
- [x] 4.6 Add `clap` color feature (2393beb)
- [x] 4.7 Atomic writes everywhere (2393beb)

### Phase 5: Error Handling Audit
- [x] 5.1 `resolve_bin_cmd` stop over-recovering (4df28fb)
- [x] 5.2 `receive.rs` remove legacy no-session fallback (4df28fb)
- [x] 5.3 `eprintln` vs `println` audit: all correct (4df28fb)
- [x] 5.4 Systematic `let _ =` audit: all annotated (4df28fb)
- [x] 5.5 Lock file consistency: unified on lockfile crate (066144d)
- [x] 5.6 `auto_upgrade_hooks`: already rate-limited by version check (066144d)

### Phase 6: Recording Daemon Improvements
- [x] 6.1 Reorder daemon startup
- [x] 6.2 Remove 200ms sleep in `spawn_daemon`
- [x] 6.3 Extract `DaemonState` struct
- [x] 6.4 Signal handler for graceful lock cleanup
- [x] 6.5 Add more commentary to audio and transcription logic

### Phase 7: Agent Trait Generalization
- [x] 7.1 Refactor hook logic into generic + agent-specific
- [x] 7.2 Split narration instructions (done by architecture: no separate code needed)
- [x] 7.3 Track project-specific installations
- [x] 7.4 Research skill format generalization (deferred: trait boundary from 7.1 is prerequisite)
- [x] 7.5 EXTENDING.md rewrite, claude asset reorg into claude/ subfolder (35571a3)

### Phase 8: UX Improvements
- [x] 8.1 Model download during `/attend` activation (f170c5c)
- [x] 8.2 Auto-cleanup with configurable retention (f170c5c)
- [x] 8.3 Cross-platform keybindings (f170c5c)
- [x] 8.4 Elided line ranges in narration output (f170c5c)
- [x] 8.5 Context line tuning for highlights (f170c5c)
- [x] 8.6 Check parakeet-rs upstream: already on 0.3.3, no action needed
- [x] 8.7 Narration quality: 500ms dwell threshold + trailing cursor-only removal (1c8d192)
- [x] 8.8 Stop hook exit code: clean exit 0 with no output (c2796cb)
- [x] 8.9 Listener restart instructions for transient failures (774ecae)
- [x] 8.10 Research custom vocabulary: see phase-08 notes
- [x] 8.11 Research agent-driven walkthrough via Zed ACP: see phase-08 notes

### Phase 9: Test Hardening
- [x] 9.1 Test documentation pass
- [x] 9.2 install/uninstall test coverage
- [x] 9.3 Prop test expansion
- [x] 9.4 Silence detector integration test

### Phase 10: merge.rs Deep Refactor
- [x] 10.1 Comprehensive test suite for merge.rs (364c25d)
- [x] 10.2 Single streaming pass rewrite (f7e8eed)
- [x] 10.3 Documentation (f150ba5)

### Phase 12a: External Context Sources (Part A)
- [x] A1 `ExternalSource` trait, `ExternalSnapshot`, `platform_source()` dispatch
- [x] A2 macOS backend (`ext_capture/macos.rs`): AX queries via `accessibility` crate
- [x] A3 New `Event::ExternalSelection` variant + serde
- [x] A4 Polling thread, `ExtDwellTracker`, dedup logic in `ext_capture.rs`
- [x] A5 Wire ext_capture into `CaptureHandle` (third thread)
- [x] A6 `render.rs`: render ExternalSelection as blockquote
- [x] A7 `merge.rs`: compress consecutive same-app selections
- [x] A8 `receive.rs`: pass ExternalSelection through filter unchanged
- [x] A9 Config: `ext_ignore_apps` with default `["Zed"]`
- [x] A10 Tests: DwellTracker unit tests, merge/compress/prop tests, render tests, receive filter test

### Phase 12b: Firefox Native Messaging (Part B)
- [x] B0 Fix snip policy: stop snipping non-reconstructable events (diffs, external selections)
- [x] B1 New `Event::BrowserSelection` variant + serde
- [x] B2 Firefox extension: content.js + background.js + manifest.json
- [x] B3 `attend browser-bridge` subcommand (native messaging host)
- [x] B4 `Browser` trait + Firefox implementation
- [x] B5 CLI `--browser` wiring (install/uninstall)
- [x] B6 `render.rs`: render BrowserSelection (code vs prose)
- [x] B7 `merge.rs`: BrowserSelection dedup + cross-type dedup with ExternalSelection
- [x] B8 Tests: 12 new tests (compress, prop, render, receive)
- [x] B9 Documentation updates
- [x] B11 Signed XPI embedding + Chrome support + xtask sign-extension + CI workflow

### Phase 13: No-Session Support
- [x] 13.1 Staging/pending dir functions: accept `Option<&SessionId>`, fall back to `"_local"`
- [x] 13.2 Daemon: pass `Option` to pending/staging dirs, remove `narration.json` fallback
- [x] 13.3 Browser bridge: use `_local` when no session but `record.lock` exists
- [x] 13.4 Receive: `collect_pending` / `read_pending` handle `_local` directory
- [x] 13.5 Tests: no-session staging, pending read from `_local`, browser bridge fallback

### Phase 14: Pause
- [x] 14.1 Pause sentinel path + `attend narrate pause` CLI subcommand
- [x] 14.2 `audio::CaptureHandle::pause()` / `resume()`
- [x] 14.3 `capture::CaptureHandle` paused flag + `pause()` / `resume()`
- [x] 14.4 Editor/diff/ext threads: check `paused` flag, skip polling when set
- [x] 14.5 `DaemonState` pause support (flush-then-suspend, resume detection)
- [x] 14.6 Pause/resume chimes + empty chime
- [x] 14.7 Wire chimes into daemon (pause, resume, empty on stop/flush)
- [x] 14.8 `narrate status`: report "paused" state
- [x] 14.9 Zed task + keybinding for pause
- [x] 14.10 Tests: pause/resume sentinel, full suspend, empty chime

### Phase 15: Shell Hook Integration
- [x] 15.1 `Event::ShellCommand` variant + serde (with cwd field for filtering)
- [x] 15.2 `Shell` trait, module layout, fish + zsh implementations
- [x] 15.3 `attend shell-hook` CLI subcommand (preexec/postexec staging)
- [x] 15.4 Fish hook + completion installation (conf.d auto-source)
- [x] 15.5 Zsh hook + completion installation (add-zsh-hook preexec/precmd)
- [x] 15.6 Generalized `StagingResult`/`StagingCleanup` + `collect_shell_staging`
- [x] 15.7 Wire shell staging into recording daemon (`transcribe_and_write`)
- [x] 15.8 `render.rs`: fenced code block with shell tag, cwd comment, exit/duration
- [x] 15.9 `merge.rs`: preexec/postexec dedup (retain-all for idempotency)
- [x] 15.10 `receive.rs`: cwd-based filtering + relativization
- [x] 15.11 CLI `--shell` wiring (install/uninstall/status/auto-upgrade)
- [x] 15.12 Tests: 21 new tests (compress, prop, render, receive filter)

### Phase 16: Yank-to-Clipboard
- [x] 16.1 `yanked_dir()` + yank sentinel path
- [x] 16.2 `check_yank()` in daemon (write to `yanked/` dir)
- [x] 16.3 `attend narrate yank` CLI subcommand
- [x] 16.4 Clipboard write via `arboard`
- [x] 16.5 Zed task + keybinding for yank
- [x] 16.6 Tests: yank output, empty yank preserves clipboard, `yanked/` cleanup

### Phase 17: Loopback Audio Capture
- [ ] 17.1 Replace webrtc-vad with webrtc-audio-processing (VAD migration)
- [ ] 17.2 Add AudioSource to the event model
- [ ] 17.3 Loopback capture stream (macOS only initially)
- [ ] 17.4 Echo cancellation wiring
- [ ] 17.5 Dual-stream transcription
- [ ] 17.6 Render and merge (`<other-speaker>` blocks)
- [ ] 17.7 Permission and UX (runtime detection, docs)
- [ ] 17.8 Tests
- [ ] **Linux loopback support** — blocked on cpal PulseAudio backend release (merged to master, not yet in a cpal release). Once available, loopback on Linux is straightforward: PulseAudio/PipeWire monitor sources are standard input devices. No attend code changes expected beyond enabling the `pulseaudio` cpal feature.

### Phase 18: Persistent Daemon
- [ ] 18.1 Benchmark model load time
- [ ] 18.2 Stop → flush+pause (daemon survives stop, enters idle)
- [ ] 18.3 IPC: wake a paused daemon (resume sentinel from toggle/start)
- [ ] 18.4 Idle timeout (configurable, default 5m)
- [ ] 18.5 Memory footprint analysis
- [ ] 18.6 Daemon health and observability
