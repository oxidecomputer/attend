# Code Review Order

Logical ordering for a complete line-by-line review of the attend codebase.
Organized by dependency order: definitions appear before usage.

## Phase 1: Foundational Types & Infrastructure

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 1 | `src/util.rs` | 111 | Atomic I/O, UTC formatting, XDG config, path inclusion |
| 2 | `src/clock.rs` | 19 | Process-wide clock factory (real vs mock) |
| 3 | `crates/mock-clock/src/lib.rs` | ~800 | Injectable clock: `Clock`, `SyncClock`, `MockClock`, `RealClock` |
| 4 | `crates/mock-clock/src/tests.rs` | 95 | Mock clock tests |
| 5 | `src/state.rs` | 518 | `SessionId`, `FileEntry`, `EditorState`, `Selection`, `Position`, `Line` |
| 6 | `src/state/resolve.rs` | 338 | Path resolution and relativization |
| 7 | `src/state/tests.rs` | 869 | State module tests |

## Phase 2: Domain Types & Protocols

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 8 | `src/config.rs` | 173 | Config loading and merging (global + hierarchical) |
| 9 | `src/config/tests.rs` | 232 | Config tests |
| 10 | `src/terminal.rs` | 183 | Terminal I/O, `AlternateScreen` RAII guard |
| 11 | `src/hook/types.rs` | 114 | `HookType`, `HookKind`, `HookInput`, `HookDecision`, `GuidanceEffect` |
| 12 | `src/hook/session_state.rs` | 132 | Per-session hook state, displacement tracking |

## Phase 3: External Integrations

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 13 | `crates/macos-disclaim/src/lib.rs` | 340 | macOS TCC disclaiming for child processes |
| 14 | `src/editor.rs` | 77 | `Editor` trait and factory |
| 15 | `src/editor/zed.rs` | 110 | Zed editor backend |
| 16 | `src/editor/zed/db.rs` | 76 | Zed SQLite workspace queries |
| 17 | `src/editor/zed/jsonc.rs` | 141 | JSONC parsing/modification for Zed settings |
| 18 | `src/editor/zed/health.rs` | 59 | Zed health checks |
| 19 | `src/editor/zed/keybindings.rs` | 79 | Zed keybinding installation |
| 20 | `src/editor/zed/tasks.rs` | 70 | Zed task configuration |
| 21 | `src/editor/zed/tests.rs` | 429 | Zed backend tests |
| 22 | `src/browser.rs` | 20 | `Browser` trait and factory |
| 23 | `src/browser/firefox.rs` | 142 | Firefox native messaging integration |
| 24 | `src/browser/chrome.rs` | 99 | Chrome native messaging integration |
| 25 | `src/shell.rs` | 44 | `Shell` trait and factory |
| 26 | `src/shell/fish.rs` | 127 | Fish shell hook/completions |
| 27 | `src/shell/zsh.rs` | 160 | Zsh shell integration |

## Phase 4: Audio & Transcription Pipeline

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 28 | `src/narrate/audio.rs` | 275 | Audio capture via CPAL, resampling, format conversion |
| 29 | `src/narrate/audio/tests.rs` | 112 | Audio tests |
| 30 | `src/narrate/silence.rs` | 181 | Silence detection via WebRTC VAD |
| 31 | `src/narrate/silence/tests.rs` | 242 | Silence detection tests |
| 32 | `src/narrate/transcribe.rs` | 150 | `Transcriber` trait, `Engine` enum, `Word` type |
| 33 | `src/narrate/transcribe/whisper.rs` | 240 | Whisper GGML backend |
| 34 | `src/narrate/transcribe/parakeet.rs` | 171 | Parakeet ONNX backend |
| 35 | `src/narrate/transcribe/stub.rs` | 187 | Stub transcriber for testing |
| 36 | `src/narrate/chime.rs` | 172 | Audio feedback tones |

## Phase 5: Capture Subsystem

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 37 | `src/narrate/capture.rs` | 566 | Capture coordinator, `Event`, `CapturedRegion` |
| 38 | `src/narrate/editor_capture.rs` | 436 | Editor state polling thread |
| 39 | `src/narrate/clipboard_capture.rs` | 281 | Clipboard monitoring |
| 40 | `src/narrate/clipboard_capture/tests.rs` | 138 | Clipboard tests |
| 41 | `src/narrate/diff_capture.rs` | 105 | File modification detection |
| 42 | `src/narrate/ext_capture.rs` | 196 | External app text capture (macOS AX) |
| 43 | `src/narrate/ext_capture/macos.rs` | 157 | macOS Accessibility framework |
| 44 | `src/narrate/ext_capture/tests.rs` | 123 | External capture tests |

## Phase 6: Event Merge & Rendering

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 45 | `src/narrate/merge.rs` | 948 | Chronological merge, compression, dedup, event algebra |
| 46 | `src/narrate/merge/tests.rs` | 3 | Test organization |
| 47 | `src/narrate/merge/tests/prop.rs` | 1019 | Property-based merge tests |
| 48 | `src/narrate/merge/tests/compress.rs` | 1821 | Snapshot-based compression tests |
| 49 | `src/narrate/merge/tests/render.rs` | 757 | Render pipeline tests |
| 50 | `src/narrate/render.rs` | 410 | Event-to-markdown rendering |
| 51 | `src/narrate/render/tests.rs` | 301 | Render tests |
| 52 | `src/view.rs` | 411 | Editor state rendering (glance/look) |
| 53 | `src/view/parse.rs` | 129 | Compact editor state parsing |
| 54 | `src/view/annotate.rs` | 336 | Line-level selection annotation |
| 55 | `src/view/detect.rs` | 333 | Language detection for syntax highlighting |
| 56 | `src/view/gfm_languages.rs` | 1138 | Generated GFM language table |
| 57 | `src/view/tests.rs` | 1372 | View tests |

## Phase 7: Recording & Delivery

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 58 | `src/narrate/record.rs` | 1282 | Recording daemon main loop, `DeferredTranscriber` |
| 59 | `src/narrate/receive.rs` | 103 | Pending narration file handling |
| 60 | `src/narrate/receive/filter.rs` | 207 | Event filtering, path scoping, redaction |
| 61 | `src/narrate/receive/listen.rs` | 283 | Receive CLI, listener lock management |
| 62 | `src/narrate/receive/pending.rs` | 77 | File collection, archival, auto-prune |
| 63 | `src/narrate/receive/tests.rs` | 856 | Receive e2e tests |
| 64 | `src/narrate/clean.rs` | 72 | Archive cleanup and retention |
| 65 | `src/narrate/status.rs` | 280 | Daemon status introspection |
| 66 | `src/narrate/tests.rs` | 922 | High-level narrate integration tests |

## Phase 8: Hook System

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 67 | `src/hook/command.rs` | 68 | Hook command line parsing |
| 68 | `src/hook/decision.rs` | 82 | Hook decision logic |
| 69 | `src/hook/upgrade.rs` | 65 | Hook version management, auto-upgrade |
| 70 | `src/hook.rs` | 448 | Hook main logic: `session_start`, `session_end`, `editor_context` |
| 71–77 | `src/hook/tests/*.rs` | ~2137 | Harness, scenario, decision, prompt, listen_cmd, prop tests |

## Phase 9: Agent Backends

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 78 | `src/agent.rs` | 93 | `Agent` trait and factory |
| 79 | `src/agent/claude.rs` | 54 | Claude Code backend coordination |
| 80 | `src/agent/claude/input.rs` | 67 | Hook input parsing from Claude Code |
| 81 | `src/agent/claude/output.rs` | 184 | Hook output formatting for Claude Code |
| 82 | `src/agent/claude/settings.rs` | 46 | Settings coordination |
| 83 | `src/agent/claude/settings/install.rs` | 240 | Hook installation into Claude Code settings |
| 84 | `src/agent/claude/settings/uninstall.rs` | 101 | Hook removal |
| 85 | `src/agent/claude/settings/tests.rs` | 437 | Settings install/uninstall tests |

## Phase 10: Testing Infrastructure

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 86 | `crates/test-harness/src/lib.rs` | ~800 | E2E harness: `Harness`, `HarnessId`, `TraceEvent` |
| 87 | `crates/test-harness/src/protocol.rs` | ~300 | Test harness IPC protocol |
| 88 | `src/test_mode.rs` | 317 | Test mode env, `InjectRouter`, stub integration |
| 89 | `src/test_mode/stubs.rs` | 168 | Stub implementations for capture sources |
| 90 | `src/test_mode/tests.rs` | 321 | Test mode integration tests |

## Phase 11: CLI & Commands

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 91 | `src/cli.rs` | 101 | Top-level CLI: `Cli`, `Command`, `Format` |
| 92 | `src/cli/narrate.rs` | 76 | `attend narrate` subcommand group |
| 93 | `src/cli/glance.rs` | 59 | `attend glance` |
| 94 | `src/cli/look.rs` | 125 | `attend look` |
| 95 | `src/cli/meditate.rs` | 24 | `attend meditate` |
| 96 | `src/cli/listen.rs` | 34 | `attend listen` |
| 97 | `src/cli/hook.rs` | 92 | `attend hook <EVENT>` |
| 98 | `src/cli/shell_hook.rs` | 102 | `attend shell-hook` |
| 99 | `src/cli/install.rs` | 290 | `attend install` / `uninstall` |
| 100 | `src/cli/browser_bridge.rs` | 175 | Native messaging host for browser extensions |
| 101 | `src/cli/completions.rs` | 22 | Shell completion generation |

## Phase 12: Watch Loop

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 102 | `src/watch.rs` | 279 | Polling loop, live display for glance/look/meditate |

## Phase 13: Entry Point & Build

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 103 | `src/main.rs` | 48 | Binary entry point |
| 104 | `build.rs` | 12 | Conditional compilation (Firefox extension) |
| 105 | `Cargo.toml` | 81 | Workspace manifest |
| 106–109 | `crates/*/Cargo.toml`, `tools/xtask/Cargo.toml` | ~80 | Sub-crate manifests |
| 110 | `tools/xtask/src/main.rs` | ~150 | Build tasks: gen-gfm-languages, sign-extension |

## Phase 14: Integration Tests

| # | File | Lines | Purpose |
|---|------|-------|---------|
| 111 | `tests/e2e.rs` | ~400 | End-to-end integration tests |
| 112 | `benches/e2e.rs` | ~200 | Criterion benchmarks |
