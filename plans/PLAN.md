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

- [x] 1: [Foundation](./phase-01-foundation.md) (003a47d)
- [x] 2: [Type Safety & Config Simplification](./phase-02-type-safety.md) (6990ff3)
- [x] 3: [Module Reorganization](./phase-03-module-reorg.md) (6733577)
- [x] 4: [Unsafe Elimination & Dependency Upgrades](./phase-04-unsafe-elimination.md) (2393beb)
- [x] 5: [Error Handling Audit](./phase-05-error-handling.md) (066144d)
- [x] 6: [Recording Daemon Improvements](./phase-06-daemon-improvements.md)
- [x] 7: [Agent Trait Generalization](./phase-07-agent-generalization.md) (35571a3)
- [x] 8: [UX Improvements](./phase-08-ux-improvements.md) (774ecae)
- [x] 9: [Test Hardening](./phase-09-test-hardening.md)
- [x] 10: [merge.rs Deep Refactor](./phase-10-merge-refactor.md)
- [x] 12a: [External Context Sources (Part A)](./phase-12-context-sources.md)
- [x] 12b: [Firefox Native Messaging (Part B)](./phase-12-context-sources.md)
- [x] 13: [No-Session Support](./phase-13-no-session.md) (0bea28a)
- [x] 14: [Pause](./phase-14-pause.md) (9729a96)
- [x] 15: [Shell Hook Integration](./phase-15-shell-hooks.md) (4bfd7aa)
- [x] 16: [Yank-to-Clipboard](./phase-16-yank.md) (56dc48b)
- [ ] 17: [Loopback Audio Capture](./phase-17-loopback-capture.md)
- [x] 18: [Persistent Daemon](./phase-18-persistent-daemon.md) — superseded by Phase 20
- [x] 19: [Clipboard Capture](./phase-19-clipboard-capture.md)
- [x] 19E: [Clipboard Image Delivery](./phase-19e-clipboard-image-delivery.md)
- [ ] 20: [Socket-Based Daemon Redesign](./phase-20-socket-daemon.md)
