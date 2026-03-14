# Codebase Improvement: Status Tracker

Claude: Please update this file every time you complete a task to ensure that
you keep track of your work.

Do not skip tasks. These were carefully planned already; if there's a good
reason why one cannot be performed, feel free to call it out, but if a task can
be performed as specified, please do it.

## General Gates (every commit)

1. `cargo fmt` — clean formatting automatically (no need to check it first, just always format)
2. `cargo clippy` — zero warnings (no new `#[allow]` without approval from user)
3. `cargo nextest run` — all tests pass, none are individually slower than 3 seconds, *no nondeterministic / flaky tests*

## Upcoming tasks (delete these as they are accomplished, and move plan files to `completed/`)

Current: [code review](phase-23-code-review.md).

Next: [loopback capture](phase-17-loopback-capture.md), [self-codesign for macOS](phase-22-self-codesign.md).
