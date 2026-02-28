# Phase 0: Test Infrastructure and Oracle Suites

**No functional changes.** This phase only adds testability and tests.

Parent: [Socket-Based Daemon Redesign](phase-20-socket-daemon.md)
Testing detail: [Test Infrastructure](phase-20-testing.md)

## Status

<!-- EXTREMELY IMPORTANT: Keep this section current. Update **BEFORE** every commit. -->

- [x] 1. Trait extraction — `3c4f05b`, `69553a6`, `023c84d`, `6ccb168`
- [x] 2. `StubTranscriber` — `3ca31b6`
- [x] 3. Clock trait and `Instant` elimination — `f2a4bf0`, `f15470a`, `633404f`, `f736e38`
- [x] 4. `ATTEND_TEST_MODE` / `ATTEND_CACHE_DIR` / inject socket — `6b1d1b0`..`3215798` (8 commits)
- [x] 5. End-to-end test harness — `086fac1`..`6722d81` (3 commits)
- [ ] 6. Differential oracle (shared run infrastructure + self-diff)
- [ ] 7. Declarative oracle (invariant assertions on RunTrace)

**Note on items 4-5:** These were completed per their original spec
(fire-and-forget broadcasts, blocking per-command harness). The oracle
design (items 6-7) requires upgrades to both the inject socket protocol
and the test harness that were designed after items 4-5 landed:

- [x] Inject socket becomes bidirectional (ACK after `AdvanceTime`) — `bf748a2`
- [x] `Connection` gains a reader for ACK messages — `bf748a2`
- [x] `Inject` split into `CaptureInject` + `TimeInject` (type-level
  enforcement: `broadcast_capture()` cannot send `AdvanceTime`) — `bf748a2`
- [x] `MockClock::wait_for_waiters()` uses condvar (not spin) — `bf748a2`
- [ ] `TestHarness` switches to all-background execution model
- [ ] `HarnessId` replaces OS PIDs in trace output

The checked items were completed as a prerequisite for item 6. The
remaining items will be implemented as part of item 6 itself. See
[testing doc](phase-20-testing.md) for the full spec.

- [x] `wait_child_ticking` uses wall-clock timeout (not mock-time timeout) — `de32820`

**Known issue:** `status_shows_recording_state` e2e test is `#[ignore]`d.
The daemon's detached-grandchild startup races `wait_child_ticking`'s
scheduling: `yield_now()` between ACK-based ticks gives insufficient
wall-clock time for the daemon to connect. The all-background execution
model will fix this.

### Bug fixes discovered during implementation

- `92f8cc7` — Fix vacuous empty-string match in clipboard dedup
- `c383639` — Fix cross-run clipboard dedup: promote to global pass
- `cd5c577` — Gate stub module behind `#[cfg(test)]`, document build verification
- `f68eaec` — Fix panic on multi-byte UTF-8 in editor snapshot annotation

## Existing traits and patterns

All capture source traits are now extracted (items 1-3 complete). The
trait boundaries and supporting infrastructure:

- **`Transcriber` trait** (`src/narrate/transcribe.rs`): `transcribe()`,
  `set_context()`, `bench()`. Whisper, Parakeet, and `StubTranscriber`
  implement it. `StubTranscriber` accepts injected text via channel.

- **`ExternalSource` trait** (`src/narrate/ext_capture.rs`):
  `is_available()` and `query()`. macOS accessibility backend; `None`
  on non-macOS.

- **`EditorStateSource` trait** (`src/narrate/editor_capture.rs`):
  `current(cwd, ignores) → Option<EditorState>`. macOS accessibility
  backend.

- **`ClipboardSource` trait** (`src/narrate/clipboard_capture.rs`):
  `get_text()`, `get_image()`. `arboard` backend.

- **`AudioSource` trait** (`src/narrate/audio.rs`): `take_chunks()`,
  `pause()`, `resume()`, `drain()`, `sample_rate()`. cpal backend.

- **`CaptureConfig`** (`src/narrate/capture.rs`): bundles `Arc<dyn
  Clock>`, `Box<dyn EditorStateSource>`, `Option<Box<dyn
  ExternalSource>>`, clipboard factory. `production(clock)` returns
  real sources; test mode substitutes stubs.

- **`Clock` trait** (`src/clock.rs`): `now() → DateTime<Utc>`,
  `sleep(Duration)`. `RealClock` for production; `MockClock` (un-gated
  in item 4, with condvar-gated sleep) for tests. `process_clock()`
  returns the process-wide clock.

- **`CacheDirGuard`** (`src/state.rs`): RAII guard that creates a temp
  dir and installs a thread-local cache dir override via `RefCell`.
  Already used by the hook test harness. This is the foundation for
  `ATTEND_CACHE_DIR` support — the env var just needs to feed into the
  same override mechanism for CLI-spawned processes.

- **Pure state machines** are already factored out and independently
  testable: `DwellTracker` (editor cursor dwell), `ExtDwellTracker`
  (external selection dedup), `ClipboardTracker` (clipboard change
  detection), `SilenceDetector` (VAD-based silence splitting). These
  need no trait extraction.

- **Hook test harness** (`src/hook/tests/harness.rs`): `TestHarness`
  struct with `MockAgent`, `Outcome` enum (Decision/Narration/
  Activation), and assertion helpers. The e2e harness should reuse its
  `Outcome` types for parsing hook output.

- **Proptest infrastructure** is mature: strategies exist for events
  (`arb_words`, `arb_cursor_snapshot`, `arb_diff`, etc. in
  `src/narrate/merge/tests/prop.rs`), hook sequences
  (`src/hook/tests/prop.rs`), and editor state
  (`src/state/tests.rs`). 486 tests total; proptest-heavy.

- **`build.rs`** currently only checks for a signed Firefox .xpi. It
  does **not** inject a commit hash. Phase A will add commit hash
  injection via `vergen-gitcl` (shells out to `git`, no C deps, fails
  gracefully with defaults when git is unavailable).

## Items

### 1. Trait extraction for capture sources — COMPLETE

See the "Existing traits and patterns" section above for the extracted
traits and `CaptureConfig` struct.

### 2. Stub transcriber — COMPLETE

`StubTranscriber` accepts injected text via an `std::sync::mpsc`
channel (unbounded — `try_recv()` drain, no backpressure) and returns
the injected words with synthetic timestamps. No model loading, no
audio processing.

### 3. Clock trait and `Instant` elimination — COMPLETE

`Clock` trait with `now()` and `sleep()`, `RealClock` for production,
`MockClock` for tests (condvar-gated sleep added in item 4). `Instant`
eliminated from all daemon internals; `SystemTime` retained for file
mtime.

### 4. `ATTEND_TEST_MODE` and `ATTEND_CACHE_DIR` env vars

`ATTEND_TEST_MODE=1` swaps in stub capture sources (via
`CaptureConfig`) and connects to the harness's inject socket.
`ATTEND_CACHE_DIR` controls the cache directory: set to a path to
use that path, or set to empty (`""`) to auto-create a random temp
directory (useful for manual testing and parallel runs). The existing
`CacheDirGuard` pattern handles the in-process override; the env
var extends this to CLI-spawned subprocesses. No behavioral change
to production code paths.

**Inject socket architecture.** The harness is the server: it binds
`$ATTEND_CACHE_DIR/test-inject.sock` and accepts connections. Every
process spawned with `ATTEND_TEST_MODE=1` (daemon and CLI commands
alike) creates a `MockClock`, connects to the inject socket at the
top of `main` (before any clock usage), and sends its PID and argv
as a JSON struct (`{"pid": N, "argv": [...]}`). A background thread
reads newline-delimited JSON messages from the socket and dispatches
them: `AdvanceTime` goes to the `MockClock`, capture injections go
to the appropriate stub channels (the daemon routes them; other
processes ignore them).

**Current state:** The inject socket is unidirectional
(harness→process). `AdvanceTime` is fire-and-forget: the harness
broadcasts and moves on without waiting for processes to settle. This
is sufficient for the existing e2e smoke tests (`tests/e2e.rs`) which
use `wait_child_ticking` (tick time in a loop until a specific child
exits). The oracle requires upgrading the protocol to be bidirectional
with ACK-after-settle semantics — see
[testing doc](phase-20-testing.md#tick-synchronization-ack-protocol).

**Spawn-connect synchronization.** After spawning a subprocess, the
harness blocks until that PID connects to the inject socket. This
ensures no events are sent until the new process is connected and
listening, eliminating races where execution time could determine
which events a subprocess sees.

**Broadcast-everything.** All inject messages are broadcast to every
connected process. The daemon routes capture injections to its stub
channels; non-daemon processes ignore them. This is simpler than
targeted delivery — the harness doesn't need to track roles.

**Condvar-gated mock sleep.** `MockClock::sleep(d)` blocks on a
condvar until `now() >= start + d`. When `advance()` is called
(from the inject socket background thread), it bumps the time and
broadcasts the condvar, waking any threads whose sleep deadline
has been met. This eliminates both CPU spin and real wall-clock
delay — threads proceed in lockstep with harness-driven time.
This is critical for proptest at thousands of iterations per second.

CLI commands (stop, start, yank) poll sentinel files with
`clock.sleep(SENTINEL_POLL_MS)`. With condvar sleep, these block
until the harness advances time. The harness advances time for all
processes simultaneously, so the daemon processes the sentinel and
the CLI's poll loop terminates — no real wall-clock delay anywhere.

### 5. End-to-end test harness — COMPLETE

`TestHarness` struct that spawns a real daemon in test mode, drives
it via real CLI subprocesses, and asserts on outputs. All IPC is real
(whatever mechanism the binary under test uses). The harness reuses
the hook test harness's `Outcome` type for parsing hook stdout/stderr.

**Current state:** The harness uses a blocking per-command model:
`run_command()` spawns a child, waits for its inject socket connection,
then ticks time until it exits. This works for sequential e2e smoke
tests but doesn't support the oracle's all-background execution model
where multiple processes run concurrently and exits are observed during
tick settlement. Item 6 will refactor the harness to the background
model described in the
[testing doc](phase-20-testing.md#test-harness).

### 6. Differential oracle

Builds the shared run infrastructure in `crates/test-harness/`: a
`run(binary, actions, synthetic_cwd) -> RunTrace` function that
executes a timestamped action sequence and returns a timestamped
trace of process exits, plus the `Action`/`ActionKind` types and
proptest strategies. The differential oracle itself is a workspace
member binary (`attend-oracle-diff`) that calls `run()` twice (two
binaries, same actions, same synthetic cwd) and asserts the normalized
`RunTrace` values match via `PartialEq`. Self-diff (same binary as
both sides) validates the harness under proptest's thousands of
iterations. Must pass green.

See [testing doc](phase-20-testing.md) for the full specification:
timestamped actions, trace events, ACK protocol, synthetic cwd,
cache dir normalization, HarnessId, and proptest strategies.

### 7. Declarative oracle

An integration test module in `crates/test-harness/` that calls
`run()` once and checks the `RunTrace` against state-machine
invariants. Parses raw stdout using the application's existing hook
output code. Must pass green against the current implementation.

See [testing doc](phase-20-testing.md#oracle-2-item-7-declarative-specification)
for invariant examples and the hook output parsing approach.

## Dependency order

```
1. Trait extraction (capture)  ──┐
2. Stub transcriber              ┤
3. Clock trait                   ├→ 4. Env vars + test-inject.sock → 5. Harness → 6. Diff oracle → 7. Decl oracle
```

Items 1-3 are complete and independent of each other. Item 7 depends on
item 6: the differential oracle builds the shared `run()` + `RunTrace`
infrastructure that the declarative oracle reuses.

Item 4 (env vars + inject socket) depends on the traits existing (done).
This is the most structurally significant item in Phase 0: it adds
cross-process time coordination via `test-inject.sock`, upgrades
`MockClock` with condvar-gated sleep, and wires `process_clock()` to
connect to the harness's inject socket in test mode. Every process
under test becomes an inject socket client. This is the one change in
Phase 0 that isn't pure refactoring.

**Gate**: both oracle suites pass reliably before proceeding.
