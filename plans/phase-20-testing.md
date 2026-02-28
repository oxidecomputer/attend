# Test Infrastructure

End-to-end testing infrastructure for the socket daemon migration.

Parent: [Socket-Based Daemon Redesign](phase-20-socket-daemon.md)
Phase 0 items: [Phase 0 Detail](phase-20-phase0.md)

## Test mode activation

An environment variable `ATTEND_TEST_MODE=1` triggers test configuration:

- **Audio and transcription**: entirely stubbed. No cpal, no sound card,
  no model loading, no network. `Inject::Speech { text, duration_ms }`
  combines what was said with how long it took — the stub transcriber
  returns the injected text directly, bypassing the real model. Chime
  playback is a no-op. This is essential for fuzzing at thousands of
  times realtime.
- **Editor capture**: replaced with a stub `EditorStateSource` that returns
  state injected via the inject socket (`Inject::EditorState`).
- **External capture (accessibility)**: replaced with a stub `ExternalSource`
  (the trait already exists) that returns scripted selections.
- **Clipboard capture**: replaced with a stub that emits scripted clipboard
  events.
- **Clipboard write (yank output)**: the `arboard` clipboard write in the
  yank path is replaced with a stub that writes to a file in the isolated
  cache dir (e.g., `test/yanked-clipboard.txt`). The harness reads this
  file to check yank output. Without this, parallel tests would clobber
  the real system clipboard.
- **Service manager**: test mode bypasses all launchd/systemd interaction
  (no plist writes, no `launchctl`, no `systemctl`). The test harness
  spawns the daemon directly as a child process.
- **Model download**: test mode never hits the network. The stub
  transcriber means the real model is never loaded, but the pre-download
  path in `attend listen` must also be suppressed when
  `ATTEND_TEST_MODE=1`.
- **Cache directory**: redirected to an isolated temp directory. The existing
  `CacheDirGuard` pattern (`state.rs`) handles this for in-process tests. For
  CLI-invoked tests, `ATTEND_CACHE_DIR` env var overrides `cache_dir()`.
- **Clock and inject socket**: `process_clock()` returns a `MockClock`
  (condvar-gated sleep) and connects to the harness's inject socket at
  `$ATTEND_CACHE_DIR/test-inject.sock`. A background thread reads
  injection messages and dispatches them. See the
  [Injection](#injection-how-to-feed-events-into-test-processes) section.

The capture sources are already behind traits (`ExternalSource`,
`EditorStateSource`, `ClipboardSource`, `AudioSource`). `CaptureConfig`
bundles them. The env var selects the stub config; production code is the
default.

---

## Oracle models

Both oracles are built on a shared `run()` function and `RunTrace` type.
The differential oracle is built first (item 6) because it validates the
harness infrastructure via self-diff before we layer invariant assertions
on top.

### Shared run infrastructure

All shared types (`RunTrace`, `TraceEvent`, `Action`, etc.) and the
`run()` function live in `crates/test-harness/`, alongside the existing
`TestHarness`.

#### Execution model: all processes are background

The harness treats **all** spawned processes uniformly: every CLI
command, hook invocation, and listener is launched as a background
child. The harness never blocks waiting for a specific process to
exit. Instead, it advances through the timestamped action sequence,
and process exits are observed as they occur during tick settlement.

The harness loop:

1. Advance mock time to the next action's timestamp (via `AdvanceTime`
   + ACK barrier).
2. Execute the action (spawn a process, inject data).
3. After each tick settlement, check all background children for exits
   via `try_wait()`. Any that exited get their stdout/stderr/exit_code
   captured and appended to the trace as `TraceEvent` entries.
4. Repeat until all actions are exhausted.
5. Final settlement: advance time until all remaining processes have
   exited (or timeout), capture their outputs, snapshot final state.

This eliminates special-casing: `listen` is just another background
process whose exit shows up in the trace whenever it happens. `toggle`
and `stop` are also background — they exit quickly (usually within one
or two ticks), and their output is captured at that point.

#### Input: timestamped actions

```rust
/// A harness action at a specific mock-time instant.
struct Action {
    /// Mock time (ms since epoch) at which to execute this action.
    /// The harness advances mock time to this instant before executing.
    t: u64,
    kind: ActionKind,
}

enum ActionKind {
    // Session lifecycle
    ActivateSession { session_id: String },
    DeactivateSession { session_id: String },

    // Recording control (spawn CLI subprocesses)
    Toggle,
    Start,
    Stop,
    Pause,
    Yank,

    // Queries (spawn CLI subprocesses)
    Status,

    // External events (spawn CLI subprocesses)
    BrowserEvent { url: String, title: String, html: String, text: String },
    ShellEvent { shell: String, cmd: String, exit_status: i32, duration_secs: f64 },

    // Delivery (spawn CLI/hook subprocesses)
    Listen { session_id: String },
    Collect { session_id: String },
    FirePreToolUse { session_id: String },

    // Capture injections (fire-and-forget, no process spawned)
    InjectSpeech { text: String, duration_ms: u64 },
    InjectSilence { duration_ms: u64 },
    InjectEditorState { files: Vec<String> },
    InjectExternalSelection { app: String, text: String },
    InjectClipboard { text: String },
}
```

`AdvanceTime` is not an action kind — it's implicit in the timestamp
gaps between actions. The harness advances mock time to `action.t`
before executing each action. If two actions share the same timestamp,
no time advance occurs between them.

#### Output: timestamped trace events

```rust
/// Harness-assigned process identifier. Sequential, deterministic.
/// The i-th process spawned by the harness gets HarnessId(i).
/// OS PIDs are nondeterministic and never appear in the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct HarnessId(u32);

/// A process exit observed during tick settlement.
struct TraceEvent {
    /// Mock time at which the exit was observed.
    t: u64,
    /// Which process exited (harness-assigned, not OS PID).
    process: HarnessId,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

/// The complete trace: timestamped process exits, plus final state.
///
/// Before comparison, the differential oracle normalizes the trace:
/// verbatim string-replacement of the run's random cache dir path
/// with a fixed placeholder. No false positives because the cache
/// dir is a randomly generated temp path.
#[derive(PartialEq)]
struct RunTrace {
    events: Vec<TraceEvent>,
    final_state: FinalState,
}

/// State observed after the full sequence completes.
struct FinalState {
    daemon_alive: bool,
    archive_contents: Vec<(String, Vec<u8>)>,
    yank_clipboard: Option<String>,
}
```

The trace is the timestamped ordered list of process exits. The input
(the timestamped action sequence) is known and identical for both runs.
Given the ACK protocol's lockstep execution guarantee, the same actions
at the same timestamps produce the same trace events at the same
timestamps — deterministically, across OS processes.

#### Synthetic cwd and cache dir normalization

Each `run()` call creates a fresh `TestHarness` with a random temp dir
as its cache. Hook calls need a `cwd` parameter for path scope filtering
and relativization. This `cwd` must NOT be the cache dir (which differs
between the two differential runs). Instead, `run()` takes a
`synthetic_cwd` parameter — a fixed path generated once per proptest
case and shared between both runs.

```rust
/// Execute a timestamped action sequence and collect the trace.
fn run(binary: &Utf8Path, actions: &[Action], synthetic_cwd: &Utf8Path) -> RunTrace { ... }
```

Injected editor/diff paths are under `synthetic_cwd` so they pass
scope filtering and get relativized consistently. The `synthetic_cwd`
can be randomized per proptest case (for diversity) but is fixed
between the two parallel `run()` calls.

After `run()` returns, the differential oracle normalizes the
`RunTrace`: verbatim string-replacement of the run's cache dir path
with a fixed placeholder (e.g., `<CACHE>`). Since the cache dir is a
randomly generated temp path, false positive replacements are
impossible.

#### Proptest shrinking

`run()` makes proptest shrinking effective: on failure, proptest
replays shorter action sequences through the same `run()`, and
the oracle compares the same `RunTrace` type. Minimal reproducing
sequences fall out naturally.

### Oracle 1 (item 6): Differential

A standalone binary (`attend-oracle-diff`) that uses proptest internally
to generate, execute, and shrink action sequences. It takes two `attend`
binary paths and fuzzes them against each other across many iterations:

```
attend-oracle-diff --binary-a ./target/release/attend-old \
                   --binary-b ./target/release/attend-new
```

For each proptest case:
1. Generate a random timestamped action sequence and a synthetic cwd.
2. Call `run(binary_a, &actions, &cwd)` and `run(binary_b, &actions, &cwd)`
   — these can run concurrently (separate `TestHarness` instances,
   separate temp dirs, separate inject sockets).
3. Normalize both `RunTrace` values (cache dir replacement).
4. Assert the two normalized traces match via `PartialEq`.
5. On mismatch: proptest shrinks the action sequence and reports the
   minimal failing case.

Each binary is a matched pair: the same build is used for both CLI
commands and the daemon. The oracle never crosses versions (e.g.,
binary-a's CLI against binary-b's daemon) — that would cause version
mismatches once Phase A adds commit-hash checking.

This is a workspace member binary crate (e.g., `crates/oracle-diff/`),
not a `#[test]`. It depends on `crates/test-harness/` for `run()` and
`RunTrace`, but has no dependency on the main `attend` crate's internals
— it only shells out to the two binaries. This means it works across any
two commits: build the old commit, build the new one, point the oracle
at both.

**Self-diff as harness validation**: Running the same binary as both sides
must produce all-green. If it doesn't, either `run()` has nondeterminism
bugs or the daemon's test mode is non-deterministic. Self-diff is the
primary harness validation tool — it stress-tests the inject socket,
ACK protocol, spawn-connect synchronization, and lockstep time
coordination under proptest's thousands of iterations. This is why the
differential oracle is built first: we need confidence in the
infrastructure before layering invariant assertions on top.

Passing self-diff is necessary but not sufficient — the `RunTrace` fields
must be tight enough that swapping in a broken binary would fail. The
comparison is `PartialEq` on the normalized `RunTrace`; the following
properties explain why byte equality catches real divergences:

- Collected narration contains exactly the same events in the same order
- Each injected transcript string appears verbatim in the output
- Each injected browser/shell/editor/ext event appears in the output
- Status report fields match (recording, paused, engine, pending count)
- Yank produces identical clipboard content
- Archive directory contains the same files with the same content
- Daemon exit behavior matches (alive vs exited, exit code)
- Process exits occur at the same mock-time instants
- Listener exits at the same tick relative to other process exits

The typical workflow during migration:

```bash
# Build baseline in a worktree (one-time setup)
git worktree add ../attend-baseline main
cargo build --release --features test-mode \
    --manifest-path ../attend-baseline/Cargo.toml

# Build current work
cargo build --release --features test-mode

# Diff them
cargo run --release --bin attend-oracle-diff -- \
    --binary-a ../attend-baseline/target/release/attend \
    --binary-b ./target/release/attend
```

The worktree stays around for the duration of the migration. Update it
with `git -C ../attend-baseline pull` as needed.

**Baseline constraint**: The older binary must have Phase 0 (test
infrastructure) already landed — it needs `ATTEND_TEST_MODE`,
`ATTEND_CACHE_DIR`, and `test-inject.sock` support. The differential
oracle compares Phase 0+ against Phase A+, not pre-Phase-0 code. This
is why Phase 0 must be fully complete and green before any migration
phase begins.

### Oracle 2 (item 7): Declarative specification

A state-machine specification that describes expected behavior
independently of either implementation. It operates on the same raw
`RunTrace` that `run()` returns, but **parses** the stdout fields
using the application's existing hook output parser to extract
structured narration content, then asserts invariants on the result.

This lives as an integration test module in `crates/test-harness/`
(e.g., `tests/oracle_spec.rs`), not a separate binary. It depends on
the main `attend` crate for the hook output parser.

For each proptest case:
1. Generate a random timestamped action sequence and a synthetic cwd.
2. Call `run(binary, &actions, &cwd)`.
3. Parse relevant `TraceEvent` stdout fields into structured narration /
   status types using application code.
4. Check the parsed results against state-machine invariants.
5. On violation: proptest shrinks and reports the minimal failing case.

Example invariants:

- "After Toggle (start) + Toggle (stop), at most one narration should be
  collectible, and if words were spoken, exactly one will be."
- "After Toggle (start) + Pause + Pause (resume) + Toggle (stop), narration
  includes events from both recording periods."
- "After Toggle (start) + BrowserEvent + Toggle (stop) + Collect, the
  delivered narration contains the browser event."
- "After Yank, clipboard is non-empty."
- "Start while already recording finalizes and resumes (flush)."
- "Stop while idle is a no-op."
- "Wait without pending narrations blocks until the next stop or start-while-recording."

These are invariants, not exact output comparisons. They can be expressed
as proptest postconditions on the `RunTrace`. This oracle survives
implementation changes (e.g., if we later change merge ordering or
timestamp precision) where the differential oracle would break.

The declarative oracle is the durable asset; the differential oracle is
the migration safety net.

---

## Injection: how to feed events into test processes

The test harness needs to inject events into the daemon (speech, editor
state, etc.) and advance time for *all* processes under test (daemon and
CLI commands alike). This is a cross-process coordination problem.

### Architecture: harness as inject server

The **harness** is the server. It binds
`$ATTEND_CACHE_DIR/test-inject.sock` before spawning any processes.

Every process spawned with `ATTEND_TEST_MODE=1` — daemon, CLI commands
(`toggle`, `start`, `stop`, `yank`, `status`), `listen`, hook processes —
connects to the inject socket as early as possible in its lifecycle (top
of `main`, before any clock usage).

### Inject socket protocol

**Framing**: newline-delimited JSON. Each message is a single JSON
object followed by `\n`. This applies in all directions (handshake
client→server, injections server→client, ACKs client→server).
Debuggable with `socat` / `jq` — same rationale as the main control
socket.

**Handshake**: on connect, the process sends a single JSON struct with
its PID and full command line (`argv`):

```json
{"pid": 12345, "argv": ["attend", "narrate", "_daemon"]}
```

The `argv` field lets the harness positively identify the daemon
connection (its argv ends with `narrate _daemon`) vs CLI commands. The
harness asserts that unknown PIDs (those not from a `Command::spawn()`
it initiated) always have daemon argv — any other unknown PID is a bug.

After the handshake, the connection is bidirectional:

- **Harness → process**: `Inject` messages (time advances, capture injections).
- **Process → harness**: `Ack` messages after processing `AdvanceTime`
  (see [Tick synchronization](#tick-synchronization-ack-protocol) below).

The connection closes when the process exits and the socket drops.

**Messages** (harness→process, one JSON object per line):

```rust
/// Harness → Process (broadcast to all connections via inject socket)
#[derive(Serialize, Deserialize)]
enum Inject {
    /// Advance the mock clock by this duration. Wakes any threads
    /// blocked in MockClock::sleep() whose deadline is now met.
    AdvanceTime { duration_ms: u64 },

    /// Inject speech: what was said and how long it took.
    /// Daemon routes to stub transcriber; others ignore.
    Speech { text: String, duration_ms: u64 },
    /// Inject a period of silence.
    /// Daemon routes to stub transcriber; others ignore.
    Silence { duration_ms: u64 },
    /// Stub editor capture returns this state on next poll.
    /// Daemon routes to stub editor source; others ignore.
    EditorState { files: Vec<FileEntry> },
    /// Stub ext capture returns this selection on next poll.
    /// Daemon routes to stub external source; others ignore.
    ExternalSelection { app: String, text: String },
    /// Stub clipboard capture emits this content on next poll.
    /// Daemon routes to stub clipboard source; others ignore.
    Clipboard { text: String },
}
```

### Spawn-connect synchronization

After spawning a subprocess, the harness blocks until that PID's
handshake arrives on the inject socket. This ensures no time advances
or injections are sent until the new process is connected and receiving
messages. The spawn-connect wait happens immediately after `spawn()`,
before any subsequent actions execute.

1. Harness calls `Command::new("attend").arg("narrate").arg("stop").spawn()`.
2. Harness blocks on "wait for PID N to connect to inject socket".
3. Child process starts, connects to inject socket at top of `main`,
   sends `{"pid": N, "argv": ["attend", "narrate", "stop"]}`.
4. Harness sees PID N, unblocks.
5. Harness proceeds with the next action.

**Daemon spawn**: some CLI commands spawn the daemon as a detached
grandchild (`toggle`/`start` call `spawn_daemon()` with
`process_group(0)` + `setsid()`). The harness doesn't know the daemon's
PID in advance — it only knows the PID of the CLI command it spawned.

The harness accepts connections from unknown PIDs, but validates them:
it checks the `argv` field and asserts that any unknown PID has daemon
argv (`narrate _daemon`). Any other unknown argv is a test bug.

The harness tracks which connection is the daemon (the one with daemon
argv that persists after CLI commands disconnect). This lets it decide
whether to wait for a daemon connection:

- **`toggle`/`start` when daemon is not connected**: the harness waits
  for the CLI command's PID, then additionally waits for one new
  connection with daemon argv. Only then does it proceed.
- **`toggle`/`start` when daemon is already connected**: the harness
  only waits for the CLI command's PID. The daemon is already
  receiving broadcasts.
- **`stop`/`yank`/`pause`/etc.**: the harness only waits for the CLI
  command's PID. No new daemon is expected.

**Process exit**: when a process exits, its inject socket connection
drops. The harness detects this (read returns EOF / write returns
EPIPE) and removes the connection from the broadcast set. This is
normal — ephemeral CLI commands connect briefly and disconnect.

### Broadcast-everything model

All inject messages are broadcast to every connected process. The daemon
routes capture injections (speech, editor state, clipboard, ext) to its
stub channels. Non-daemon processes ignore these — they have no stub
channels to route to. Time advances are meaningful to everyone.

This is simpler than targeted delivery: the harness doesn't need to
track which connection is the daemon vs a CLI command. It just writes
the same message to all connections.

### Tick synchronization: ACK protocol

Background processes (daemon, listener) run concurrently with the
harness. Without synchronization, the harness might execute the next
action before a background process has finished processing the
previous time tick — a source of nondeterminism that would break the
differential oracle.

**Key insight: all injected events are tick-driven.** Capture
injections (speech, editor state, clipboard, external selection) just
stage data in shared state (`Arc<Mutex<Option<T>>>`) or an mpsc
channel (`try_recv()` drain). No thread wakes. The data is only
consumed when the relevant capture thread wakes from `clock.sleep()`
on the next time tick. Therefore, only `AdvanceTime` can change
observable state — and we need the harness to wait for all processes
to fully settle after each tick before proceeding.

**Protocol:**

1. Harness sends `AdvanceTime { duration_ms }` to all connected processes.
2. Each process's inject-socket reader thread:
   a. Calls `clock.advance_and_settle(duration)` — bumps time,
      counts threads whose deadlines are met, wakes all sleeping
      threads, then blocks on a settlement condvar until all woken
      threads have re-entered `sleep()`.
   b. Writes `{"ack": true}\n` back to the harness on the inject socket.
3. Harness waits for ACK (or connection drop) from every connected process.
4. Only then proceeds to the next action.

**How `advance_and_settle` works.** The mock clock tracks registered
deadlines (one per sleeping thread). On advance:
1. Lock the clock state, bump time.
2. Count how many registered deadlines are now <= the new time (`expected`).
3. Reset `settled = 0`.
4. Broadcast the sleep condvar (wakes all sleeping threads; those whose
   deadline isn't met re-block immediately).
5. Wait on the settlement condvar until `settled >= expected`.

Each woken thread exits `sleep()`, does its per-tick work, and calls
`sleep()` again with a new deadline. Inside the new `sleep()` call,
`settled` is incremented and the settlement condvar is notified. Once
all `expected` threads have re-entered `sleep()`, `advance_and_settle`
returns.

**Daemon threads that sleep** (all gated by `clock.sleep()`):
1. Main loop — `DAEMON_LOOP_POLL_MS` (sentinel polling, audio ingest)
2. Editor capture — `EDITOR_POLL_MS`
3. Diff capture — `FILE_DIFF_POLL_SECS`
4. Ext capture — `EXT_POLL_MS`
5. Clipboard capture — `CLIPBOARD_POLL_MS` (or `PAUSED_POLL_MS` when paused)

Not all threads wake on every tick — only those whose deadline was met.
`expected` reflects only the woken threads, so threads with far-future
deadlines don't affect settlement.

**Invariant: woken threads must re-enter `sleep()`.** If a woken
thread exits its loop (process exit, thread shutdown) without calling
`clock.sleep()` again, `settled` never reaches `expected`, and
`advance_and_settle` blocks forever. The harness's `ACK_TIMEOUT`
(10s) is the safety net, but the real fix is ensuring all threads that
participate in the clock keep re-entering sleep until the shared
`stop` flag is set. See "Known limitation" below for the deeper fix.

**Process exit (implicit ACK).** If a thread wakes and the process
exits (e.g., listener finds pending files → `main()` returns →
process terminates), `advance_and_settle` blocks — but the process
exit kills all threads, closing the inject socket. The harness detects
the connection drop and treats it as an implicit ACK.

**Capture injections need no ACK.** They write to shared state
instantly. No thread wakes, no work happens, no settling needed. The
harness can send them fire-and-forget.

**Result:** The harness is the single sequencer. No process advances
its state except when the harness explicitly ticks time. This gives
true lockstep execution across OS processes, making the differential
oracle's `PartialEq` comparison sound.

### Condvar-gated mock sleep

`MockClock::sleep(d)` blocks on a condvar until `now() >= start + d`.
When the reader thread receives `AdvanceTime` and calls `advance()`,
it bumps the internal time and broadcasts the condvar. All sleeping
threads wake, check if their deadline is met, and either return or
re-block. Then the reader thread calls `wait_for_waiters(n)` to
confirm all woken threads have settled, and sends ACK to the harness.

This eliminates both CPU spin (no-op sleep) and real wall-clock delay.
Threads proceed in lockstep with harness-driven time. This is critical
for proptest at thousands of iterations per second.

Example: CLI `stop()` polls a sentinel file with
`clock.sleep(100ms)` in a loop. With condvar sleep, the thread blocks.
The harness sends `AdvanceTime { 100 }` to all processes and waits for
ACKs. The daemon's main loop wakes, sees the stop sentinel, deletes it,
re-enters sleep — the daemon's reader thread detects settlement and
sends ACK. The CLI's sleep also wakes (100ms deadline met), checks the
sentinel, finds it gone, and exits — the CLI process exits, its
connection drops, and the harness treats this as implicit ACK. No real
wall-clock time elapsed, and the harness knows all processes have
settled before proceeding.

### Clock internals

The mock clock is `Arc<MockClockInner>`:
```rust
struct MockClockInner {
    state: Mutex<ClockState>,    // current time + registered deadlines
    condvar: Condvar,            // sleep/wake coordination
    settlement: Condvar,         // signaled when a thread re-enters sleep()
    settlement_state: Mutex<SettlementState>,
}

struct ClockState {
    time: DateTime<Utc>,
    deadlines: Vec<DateTime<Utc>>,  // one per sleeping thread
}

struct SettlementState {
    settled: usize,   // threads that re-entered sleep since last advance
    expected: usize,  // threads woken by last advance
}
```

`advance_and_settle(d)` bumps time, counts deadlines <= new time
(`expected`), resets `settled = 0`, broadcasts the condvar, and
blocks on `settlement` until `settled >= expected`. This is the
ACK protocol primitive: the inject socket reader thread calls it
on each `AdvanceTime`, then sends `{"ack":true}` to the harness.

`sleep(d)` computes `deadline = now + d`, registers the deadline,
increments `settled` and notifies the settlement condvar (signaling
"I'm in sleep now"), then blocks on the condvar until `time >= deadline`.
On exit, deregisters the deadline.

Time advances only from one source: `AdvanceTime` injected by the
harness. The harness is in full control — it injects events, advances
time by known amounts, and checks results. No process autonomously
decides "5 minutes have passed"; the harness says so.

**Invariant: the inject socket background thread must never call
`clock.sleep()`.** It is the only thread that calls
`advance_and_settle()`. If it blocked on the clock condvar, no thread
could wake it — deadlock. `advance_and_settle()` blocks on the
*settlement* condvar (a different mechanism) — this is safe because
sleeping threads will re-enter `sleep()` or the process will exit.

Browser and shell events don't need injection — they're already external
CLI commands (`attend browser-bridge`, `attend shell-hook`) that the
harness invokes directly. These processes still connect to the inject
socket (for time advances) and the harness still waits for them to
connect before proceeding.

---

## Observation: the `RunTrace`

Both oracles are black-box: they only observe externally visible behavior,
captured as raw data in the `RunTrace` returned by `run()`.

| `RunTrace` field | Source |
|-----------------|--------|
| `TraceEvent { t, process, stdout, stderr, exit_code }` | Each process exit observed during tick settlement |
| `FinalState::daemon_alive` | PID / process status after sequence |
| `FinalState::archive_contents` | Files in `archive/` in isolated cache dir |
| `FinalState::yank_clipboard` | Stub clipboard file in cache dir |

The trace is a timestamped ordered list of process exits. The input
(the timestamped action sequence) is the same for both runs and is not
part of the trace. One might choose to view these two lists interleaved
for debugging, but the trace is the object of comparison.

The **differential oracle** normalizes the two traces (cache dir
replacement) and compares via `PartialEq` — raw byte comparison, no
parsing needed. If the same timestamped action sequence produces
different trace events, that's a failure.

The **declarative oracle** parses `TraceEvent` stdout fields using the
application's hook output parser (the same code that produces `Outcome`
types in `src/hook/tests/harness.rs`) to extract structured narration
content, status reports, etc. Invariants are asserted on the parsed
result.

The harness cannot observe daemon-internal state (in-memory buffers,
thread state, model load status). This is intentional — the `RunTrace`
captures the contract, not the implementation.

---

## Test harness

The `TestHarness` (already implemented in `crates/test-harness/`) drives
a single `attend` binary. The oracle's `run()` function wraps it — creating
a harness, executing actions, and collecting outputs into a `RunTrace`.

A background accept thread runs a blocking accept loop on the inject
socket. Each new connection is read for its handshake (PID + argv),
then inserted into shared state behind `Arc<(Mutex<SharedState>, Condvar)>`.
The foreground test thread waits on the condvar for specific PIDs or
daemon connections.

```rust
struct TestHarness {
    binary: Utf8PathBuf,
    _cache_dir: TempDir,
    cache_path: Utf8PathBuf,
    /// Shared with the background accept thread.
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    /// Next harness-assigned process ID (sequential, deterministic).
    next_id: u32,
    /// Background children not yet exited, keyed by OS PID.
    children: HashMap<u32, TrackedChild>,
}

struct TrackedChild {
    harness_id: HarnessId,
    child: Child,
}

struct SharedState {
    connections: HashMap<u32, Connection>,
    daemon_pid: Option<u32>,
}

struct Connection {
    writer: BufWriter<UnixStream>,
    reader: BufReader<UnixStream>,  // for reading ACKs
    argv: Vec<String>,
}
```

**Key harness methods:**

```rust
impl TestHarness {
    fn new(binary: impl Into<Utf8PathBuf>) -> Self { ... }

    // --- Spawn (all processes are background) ---

    /// Spawn an attend subcommand as a background child.
    /// Returns its HarnessId. The child is tracked until it exits.
    fn spawn(&mut self, args: &[&str]) -> HarnessId { ... }
    fn spawn_with_stdin(&mut self, args: &[&str], stdin: &str) -> HarnessId { ... }

    // --- Time and injection ---

    /// Advance mock time. Sends AdvanceTime to all processes,
    /// waits for ACK (or connection drop) from each. Then checks
    /// all children for exits via try_wait().
    fn advance_time(&mut self, duration_ms: u64) -> Vec<TraceEvent> { ... }

    /// Capture injections (fire-and-forget, no ACK needed).
    fn inject_speech(&mut self, text: &str, duration_ms: u64) { ... }
    fn inject_silence(&mut self, duration_ms: u64) { ... }
    fn inject_editor_state(&mut self, files: Vec<FileEntry>) { ... }
    fn inject_external_selection(&mut self, app: &str, text: &str) { ... }
    fn inject_clipboard(&mut self, text: &str) { ... }

    // --- Observation ---

    /// Check all background children for exits. Returns trace
    /// events for any that exited since the last check.
    fn collect_exits(&mut self, t: u64) -> Vec<TraceEvent> { ... }
}
```

Each CLI invocation is a real `std::process::Command` call to `self.binary`,
with `ATTEND_TEST_MODE=1` and `ATTEND_CACHE_DIR=<temp>` set. This exercises
the full code path: argument parsing, IPC (whatever mechanism the binary
uses), daemon handling, response serialization.

The harness is agnostic to the IPC mechanism — it only cares about CLI
inputs and observable outputs. This is what makes the differential oracle
work across implementation boundaries.

**`advance_time` is the synchronization point.** It sends `AdvanceTime`
to all connected processes, waits for ACK (or connection drop) from
each, then calls `collect_exits()` to capture any process exits that
occurred during the tick. The returned `Vec<TraceEvent>` is appended
to the trace. This is the only point where the harness observes exits —
ensuring that exit detection is deterministic and tied to specific
mock-time instants.

**Harness lifecycle per test case:** Each proptest case gets a fresh
`TestHarness`. The daemon starts when the first toggle/start action
executes (not at harness creation). On teardown, the harness sends
SIGTERM to the daemon if one is connected. To keep this fast, the
daemon in test mode should start up quickly (no model loading, no real
audio init). If startup latency is still a concern, the harness could
reuse a daemon across cases by sending a `Reset` inject command that
clears all state — but start with the simple approach first.

**How `Collect` works:** The harness always invokes the real
`attend hook pre-tool-use --agent claude` as a subprocess and captures
stdout. It never bypasses the CLI to send `Command::Collect` directly —
that would skip the hook logic (filtering, redaction, rendering) and
defeat the purpose of end-to-end testing.

Claude Code passes hook context via **stdin JSON**, not arguments. The
harness must simulate this accurately. The `HookInput` struct (defined
in `src/hook/types.rs`) has three fields: `session_id`, `cwd`, and
`kind` (which carries `bash_command` for tool-use hooks, `prompt` for
user-prompt hooks). The harness feeds this JSON on stdin when invoking
hooks. Key proptest considerations:

- `session_id`: constant within a test case (simulating one session),
  but randomly generated per case (a UUID, as in real Claude Code).
- `cwd`: the `synthetic_cwd` passed to `run()` — NOT the cache dir.
- `bash_command`: for PreToolUse on `attend listen`, this must match
  the exact binary path — the hook uses it to detect listen commands.
- `prompt`: for UserPrompt hooks, `/attend` and `/unattend` are the
  magic strings.

The Claude Code hook documentation should be consulted for the exact
stdin JSON schema to ensure the harness generates realistic inputs.

Same principle applies to every harness method: always go through the
real CLI commands and hooks, never shortcut to socket commands.

**Reusing the hook oracle.** The existing hook test harness
(`src/hook/tests/harness.rs`) already has types for hook outcomes:
`Outcome` (Decision/Narration/Activation), `HookDecision` (approve/
block with guidance reason), and assertion helpers (`assert_narration`,
`assert_decision`, `assert_activation`). The declarative oracle should
parse raw hook stdout/stderr (from `TraceEvent` fields) back into these
same types and reuse the assertion vocabulary. This avoids reimplementing
hook response parsing and keeps the e2e invariants consistent with the
unit-level hook tests.

The hook outputs structured text (guidance for the agent, narration
content in `<narration>` tags, blocking messages). The declarative
oracle needs a parser that maps this output back to `Outcome` variants.
This parser should live in a shared module accessible to both the hook
unit tests and the declarative oracle's integration tests.

---

## Proptest fuzzing

The proptest strategies generate timestamped `Action` sequences
(using the `Action` and `ActionKind` types defined in the shared run
infrastructure section above). Timestamps are monotonically increasing
mock-time instants. `AdvanceTime` is not an action kind — it's implicit
in the timestamp deltas.

**Structured sequence generation.** Raw random interleaving produces
mostly nonsensical sequences (Collect before ActivateSession, Yank
while idle). The proptest strategy should generate *structured*
sequences with random perturbations:

1. Activate a session (random UUID, constant within the sequence).
2. Start or Toggle (start recording).
3. Random interleaving of injections and external events, with
   increasing timestamps.
4. Stop or Toggle (stop) or Yank.
5. Listen + Collect.
6. Optionally repeat 2-5.
7. Optionally DeactivateSession.

Each step can be randomly omitted or reordered to test error handling
and edge cases (e.g., Start while recording triggers flush, Stop while idle is a no-op,
Collect without prior Toggle, double Toggle, Listen without
ActivateSession). The structured shape biases toward realistic
sequences while still exploring edge cases. Proptest shrinking will
strip away the random perturbations to find minimal failing cases.

**Two fuzzer strategies, both run by each oracle:**

- **Structured**: the biased strategy above. Finds bugs in the happy
  path and realistic edge cases.
- **Totally random**: `proptest::collection::vec(any::<ActionKind>(), 1..50)`
  with timestamps assigned as increasing multiples of a random step
  size. Finds bugs in error handling, unexpected command ordering,
  and state machine robustness (double yank, collect before activate,
  pause while idle, etc.). Most sequences will be nonsensical — that's
  the point.

Both oracles use the same `run()` function and the same proptest
strategies. The difference is only in what they assert on the `RunTrace`:

- **Differential**: `normalize(run(a, &actions, &cwd)) == normalize(run(b, &actions, &cwd))`
- **Declarative**: `invariants(&actions, &run(binary, &actions, &cwd))`

Declarative invariants checked after each sequence:
- No panics in daemon (process alive or exited cleanly)
- No orphaned temp files outside expected dirs
- Delivered narrations contain all injected events (no loss)
- State is consistent (not recording + not paused after stop)
- Injected transcript text appears in collected narrations

---

## What this replaces vs complements

The e2e suite **complements** (does not replace) the existing unit and proptest
suites:

| Existing suite | What it tests | Stays? |
|----------------|---------------|--------|
| merge pipeline proptests | Event ordering, dedup, timestamp logic | Yes |
| DwellTracker proptests | Cursor dwell state machine | Yes |
| hook oracle proptests | Hook delivery filtering and dedup | Yes |
| editor capture unit tests | Editor query parsing | Yes |
| view proptests | Narration rendering | Yes |

The e2e suite adds: full-pipeline integration (CLI → IPC → daemon → capture
stubs → merge → deliver via listener + hook), action sequence fuzzing, and
differential testing across implementation strategies.

---

## Resolved: settlement tracking for departing threads

### Problem (resolved)

`advance_and_settle` counted woken threads from the deadline registry
and waited for all of them to re-enter `clock.sleep()`. If a woken
thread exited its loop instead of re-sleeping, `settled` never reached
`expected`, and `advance_and_settle` blocked until `ACK_TIMEOUT`.

### Solution: participant clocks with departure tracking

The `Clock` trait gained `fn for_thread(&self) -> Arc<dyn Clock>`.
`MockClock::for_thread()` returns a `ParticipantMockClock` that wraps
the shared clock and signals departure on drop. When a thread exits
(normally or via panic), its participant clock drops, incrementing a
monotonic `departed` counter in `SettlementState`. `advance_and_settle`
snapshots the counter before waking threads and waits for
`settled + departures_since_snapshot >= expected`.

`spawn_clock_thread()` is the standard way to spawn a clock-aware
thread: it calls `for_thread()` and passes the participant clock to
the closure. All four capture threads (editor, diff, ext, clipboard)
use it. The daemon main loop calls `clock.for_thread()` directly.

Backward compatible: if `for_thread()` is never called, `departed`
stays 0, and the condition reduces to `settled >= expected`.

### Future improvement: condvar-based pause/resume

Currently all capture threads poll their `paused` and `stop` flags
via `AtomicBool` loads at the top of each loop iteration. State
changes take effect only when the thread wakes from its current
`clock.sleep()`. Replacing the atomic flags with a condvar-based
`CaptureControl` would make state transitions immediate. Low
priority — current polling works correctly.
