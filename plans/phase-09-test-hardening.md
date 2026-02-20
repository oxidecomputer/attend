# Phase 9: Test Hardening

**Dependencies**: Phase 3 (modules stable — don't want to document tests that are about to move).
**Effort**: Large | **Risk**: None

---

## 9.1 Test documentation pass

- Add `///` doc comment to every `#[test]` function stating the invariant in English
- For each test, verify the body actually exercises what the name implies
- Flag any tests that are vacuously true or testing implementation details

## 9.2 install/uninstall test coverage

- Comprehensive tests for `claude.rs` JSON manipulation:
  - Empty file, existing hooks, duplicate entries, malformed JSON
  - Project-specific vs global install
  - Uninstall leaves other hooks intact
- Same for `zed/keybindings.rs` and `zed/tasks.rs` after Phase 4.4 JSONC rewrite

## 9.3 Prop test expansion

- For each unit test, ask: "Can this invariant be stated as a property over arbitrary inputs?"
- Priority targets: `merge.rs` event compression, `state/resolve.rs` offset resolution, `view/` rendering
- Expand existing prop tests in `resolve.rs` — audit for tightness (testing the right properties, not mirroring the implementation)

## 9.4 Silence detector integration test

- Synthesize audio with speech-like signal + silence gaps
- Verify split points fire at correct instants
- Verify no splits during continuous signal

---

## Verification

- Every `#[test]` function has a `///` doc comment (enforce with a grep: `grep -B1 '#\[test\]' src/**/*.rs` and verify each has a preceding `///` line)
- Test count has increased (track before/after)
- `cargo test` still passes — new tests don't break, existing tests weren't weakened
- No test is `#[ignore]`d without a tracked reason
