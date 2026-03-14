# Phase 21: Test Harness Extraction

**Dependencies**: Phase 0 items 1-5 (test infrastructure, ACK protocol,
all-background execution model).
**Effort**: Medium | **Risk**: Low (all refactoring, no behavioral changes)

---

## Motivation

The test harness (`crates/test-harness/`) already has **zero attend
dependencies**. Its Cargo.toml lists only camino, nix, serde, serde_json,
and tempfile. The core abstractions — process spawning with
spawn-connect synchronization, deterministic `HarnessId` assignment,
lockstep time coordination via the ACK protocol, `TraceEvent` collection
— are generic to any multi-process application with a long-lived daemon
and short-lived CLI commands.

The `MockClock` and settlement protocol (`src/clock.rs`) are similarly
generic: condvar-gated sleep, `advance_and_settle()`, participant
departure tracking via `ParticipantMockClock`. No attend knowledge.

But reusability is currently blocked by three kinds of coupling:

1. **Dual type definitions.** The harness defines its own `Inject` enum
   (protocol.rs, composed via `#[serde(untagged)]` over `TimeInject` /
   `CaptureInject`) and the process side defines a separate flat `Inject`
   enum (test_mode.rs). They must serialize to identical JSON. The only
   guard is a same-type serde round-trip test — there is no cross-type
   compatibility test. A field added to one side but not the other would
   fail silently at runtime.

2. **Hardcoded application knowledge in the harness.** Env var names
   (`ATTEND_TEST_MODE`, `ATTEND_CACHE_DIR`), daemon detection (argv
   matching for `_record-daemon`), and capture injection methods
   (`inject_speech`, `inject_clipboard`, etc.) are all attend-specific
   but live in the generic harness crate.

3. **The `MockClock` lives in the main crate** (`src/clock.rs`) rather
   than in a shared crate. It can't be depended on by the harness or by
   other applications without pulling in all of attend.

This phase eliminates these couplings in four items, ordered by
dependency. Each item is independently committable and leaves tests
green.

---

## Status

<!-- EXTREMELY IMPORTANT: Keep this section current. Update **BEFORE** every commit. -->

- [ ] 1. Internal harness simplification (broadcast helper, dead-connection cleanup)
- [ ] 2. Shared wire protocol crate (`crates/wire/`)
- [ ] 3. MockClock extraction (`crates/mock-clock/`)
- [ ] 4. Generic `TestHarness<C>` and process-side `InjectHandler` trait

---

## Items

### 1. Internal harness simplification

**No new crates.** Factor duplicated code within `crates/test-harness/`
and `src/test_mode.rs`.

#### 1a. `broadcast_line` helper

`advance_time()` and `broadcast_capture()` both do: serialize to JSON
line, write to all connections, collect dead PIDs, remove dead
connections. The only difference is that `advance_time` then reads ACKs.

Extract a private method:

```rust
/// Write a line to all connections, return PIDs whose writes failed.
fn broadcast_line(state: &mut SharedState, line: &[u8]) -> Vec<u32>
```

`advance_time` calls `broadcast_line`, then reads ACKs and collects
additional dead PIDs from ACK failures. `broadcast_capture` calls
`broadcast_line` and is done.

#### 1b. `purge_connections` helper

Dead-connection removal (check daemon_pid, remove from HashMap) is
duplicated in `advance_time`, `broadcast_capture`, `collect_exits`, and
`remove_connection`. Consolidate into:

```rust
fn purge_connections(state: &mut SharedState, dead: &[u32])
```

All four call sites delegate to this.

#### 1c. Move `inject_*` convenience methods to an extension trait

`inject_speech`, `inject_silence`, `inject_editor_state`,
`inject_external_selection`, and `inject_clipboard` are attend-specific
wrappers over `broadcast_capture`. Move them to an extension trait (or
inherent methods on an attend-specific wrapper type) so the core harness
doesn't mention capture message types:

```rust
/// Attend-specific injection helpers.
pub trait AttendInject {
    fn inject_speech(&mut self, text: &str, duration_ms: u64);
    fn inject_silence(&mut self, duration_ms: u64);
    fn inject_editor_state(&mut self, files: Vec<FileEntry>);
    fn inject_external_selection(&mut self, app: &str, text: &str);
    fn inject_clipboard(&mut self, text: &str);
}
```

E2e tests and oracle code import this trait. The core harness exposes
only `broadcast_capture`.

This can also be deferred to item 4 where `broadcast_capture` becomes
generic — at that point the extension trait is the natural place for
these methods.

---

### 2. Shared wire protocol crate (`crates/wire/`)

A new workspace member crate that both the harness and the main binary
depend on. It owns the canonical definitions of all inject socket types.

**Contents:**

```
crates/wire/
├── Cargo.toml          # deps: serde, camino
└── src/
    └── lib.rs
```

**Types moved here:**

- `Handshake { pid: u32, argv: Vec<String> }` — currently defined
  separately in `src/test_mode.rs` (process side) and
  `crates/test-harness/src/protocol.rs` (harness side).

- `TimeInject` — the `AdvanceTime { duration_ms }` message. Defined once.

- `CaptureInject` — the attend-specific capture variants (Speech,
  Silence, EditorState, ExternalSelection, Clipboard). Defined once.

- `Inject` — the composed enum. Currently the harness uses
  `#[serde(untagged)]` composition, the process side uses a flat enum.
  With a single definition, this distinction disappears.

- `Ack { ack: bool }` — currently an ad-hoc JSON literal
  `{"ack":true}\n` written by both sides. Making it a real type
  improves clarity.

- `FileEntry`, `Selection`, `Position`, `Line`, `Col` — currently
  duplicated between `crates/test-harness/src/protocol.rs` (harness's
  own copies) and `src/state/resolve.rs` (main crate). The wire crate
  owns the canonical definitions; the main crate's `state::FileEntry`
  either re-exports from here or converts via `From`.

**What this eliminates:**

- The "mirror" pattern in protocol.rs (which explicitly documents
  "mirrors `attend::test_mode::Inject`" / "mirrors
  `attend::state::FileEntry`"). No more manual invariant that two types
  must serialize identically.

- The `#[serde(untagged)]` composition trick. With one `Inject` enum
  shared between both crates, there's no need for the harness to
  compose `TimeInject` / `CaptureInject` via serde magic.

- The risk of one-sided field additions. Compile error instead of
  runtime deserialization failure.

**What it preserves:**

- The type-level split between `CaptureInject` (fire-and-forget) and
  `TimeInject` (ACK-gated). The harness's `broadcast_capture` takes
  `&CaptureInject`; `advance_time` constructs `TimeInject` internally.
  The compiler still prevents accidentally broadcasting a time advance
  without ACK handling.

**Migration path:**

1. Create `crates/wire/` with the canonical types.
2. Update `crates/test-harness/Cargo.toml` to depend on `attend-wire`.
3. Replace `protocol.rs` types with re-exports from `attend-wire`.
   `protocol.rs` becomes a thin re-export module (or is deleted, with
   imports updated).
4. Update `src/test_mode.rs` to use `attend-wire::Inject` instead of
   its own enum. The `InjectRouter::dispatch` match arms stay the same;
   only the type path changes.
5. Decide on `FileEntry` integration: either `state::FileEntry`
   re-exports `attend_wire::FileEntry`, or the main crate implements
   `From<attend_wire::FileEntry> for state::FileEntry`. The former is
   simpler if the fields are identical (they are today).

**Naming.** `attend-wire` is attend-specific because `CaptureInject`
contains attend's capture message types. This is intentional: the wire
protocol *is* application-specific. What becomes generic is the
*harness* (item 4), which is parameterized over the capture message type.

---

### 3. MockClock extraction (`crates/mock-clock/`)

Move `Clock`, `RealClock`, `MockClock`, `ParticipantMockClock`,
`MockClockInner`, `ClockState`, `SettlementState`, and
`spawn_clock_thread` from `src/clock.rs` to a new workspace crate.

**Contents:**

```
crates/mock-clock/
├── Cargo.toml          # deps: chrono
└── src/
    └── lib.rs
```

**What stays in `src/clock.rs`:**

Only `process_clock()`, which calls `crate::test_mode::clock()` and
falls back to `RealClock`. This is the application-level wiring that
decides which clock to use. It becomes:

```rust
pub fn process_clock() -> Arc<dyn mock_clock::Clock> {
    if let Some(clock) = crate::test_mode::clock() {
        clock
    } else {
        Arc::new(mock_clock::RealClock)
    }
}
```

**Why a separate crate?** The `MockClock` is independently useful for
any multi-threaded Rust application that needs deterministic time in
tests. It has exactly one dependency (chrono, for `DateTime<Utc>`). It
doesn't need to know about inject sockets, harnesses, or capture
sources.

The test harness crate gains a dependency on `mock-clock` but uses it
only in its public API docs and test helpers — the harness itself never
constructs a `MockClock` (that's the process side's job). However,
making the clock a shared crate means the harness could optionally
provide in-process test utilities (single-process `run()` without
spawning subprocesses) in the future.

**Test migration:** `src/clock/tests.rs` moves to
`crates/mock-clock/src/tests.rs`. The `test_mode/tests.rs` integration
test that exercises `init() + connect() + AdvanceTime → clock.now()`
stays in the main crate since it tests the application-level wiring.

---

### 4. Generic `TestHarness<C>` and process-side `InjectHandler`

The final item: parameterize the harness over application-specific
behavior so it can drive any binary that speaks the inject socket
protocol.

#### 4a. Harness configuration

Replace hardcoded values with a builder:

```rust
pub struct HarnessConfig {
    /// Path to the binary under test.
    pub binary: Utf8PathBuf,
    /// Environment variables to set on spawned processes.
    /// Currently: ATTEND_TEST_MODE=1, ATTEND_CACHE_DIR=<temp>.
    pub env: Vec<(String, String)>,
    /// Predicate: does this handshake's argv identify a daemon?
    /// Default: always false (no daemon concept).
    pub is_daemon: Box<dyn Fn(&[String]) -> bool + Send>,
    /// Predicate: does spawning this command also spawn a daemon?
    /// Args: (spawn args, has_daemon_already). Default: always false.
    pub spawns_daemon: Box<dyn Fn(&[&str], bool) -> bool + Send>,
}
```

`TestHarness::new` takes `HarnessConfig`. The existing
`TestHarness::new(binary)` becomes a convenience constructor that fills
in attend-specific defaults (or is replaced by an attend-specific
builder in the e2e test helpers).

The config's `env` field replaces the hardcoded `ATTEND_TEST_MODE` /
`ATTEND_CACHE_DIR`. The harness always adds `CACHE_DIR=<temp>` (using
a generic env var name, or letting the application name it). The test
mode env var is application-specific and goes in `env`.

**Detail: cache dir env var.** The harness creates the temp dir and
needs to communicate its path to spawned processes. Today this is
`ATTEND_CACHE_DIR`. For genericity, the harness can inject a
well-known env var (e.g., the first env entry, or a dedicated
`cache_dir_env` field) with the temp dir path. Or it can simply
expose `cache_dir()` and let the application's config builder
populate `env` with it. The latter is simpler and more explicit.

#### 4b. Generic capture broadcast

```rust
pub struct TestHarness<C: Serialize = ()> {
    // ... existing fields ...
    _capture: PhantomData<C>,
}

impl<C: Serialize> TestHarness<C> {
    /// Broadcast a capture injection to all connected processes
    /// (fire-and-forget, no ACK). The message is serialized as a JSON
    /// line and written to every connection.
    pub fn broadcast_capture(&self, msg: &C) { ... }
}
```

`advance_time`, `spawn`, `collect_exits`, `tick_until_exit`,
`has_daemon` are all independent of `C` and work unchanged.

The attend-specific type becomes: `TestHarness<CaptureInject>`.
Applications without capture injection use `TestHarness<()>` (or
`TestHarness` via the default) and never call `broadcast_capture`.

The `inject_*` convenience methods become an extension trait on
`TestHarness<CaptureInject>`:

```rust
pub trait AttendInject {
    fn inject_speech(&mut self, text: &str, duration_ms: u64);
    // ...
}

impl AttendInject for TestHarness<CaptureInject> {
    fn inject_speech(&mut self, text: &str, duration_ms: u64) {
        self.broadcast_capture(&CaptureInject::Speech {
            text: text.to_string(),
            duration_ms,
        });
    }
    // ...
}
```

#### 4c. Process-side `InjectHandler` trait

The reader loop in `src/test_mode.rs` has a fixed structure: read JSON
lines, handle time advances (advance_and_settle + ACK), dispatch
everything else to application-specific code. Extract the generic
skeleton:

```rust
/// Application-specific handler for non-time inject messages.
///
/// Implementors receive deserialized capture messages and route them
/// to the appropriate stubs or channels. The reader loop handles
/// `AdvanceTime` (clock settlement + ACK) automatically.
pub trait InjectHandler: Send + 'static {
    /// The application's capture message type. Must deserialize from
    /// the same JSON that the harness serializes via `broadcast_capture`.
    type Msg: DeserializeOwned + Send;

    /// Dispatch a capture message. Called from the reader thread.
    fn handle(&self, msg: Self::Msg);
}

/// Connect to the harness inject socket, send the handshake, and spawn
/// the background reader thread.
///
/// The reader thread calls `clock.advance_and_settle()` on `AdvanceTime`
/// messages and `handler.handle()` on everything else. It never calls
/// `clock.sleep()` (invariant: the reader is the only thread that
/// calls `advance()`).
pub fn connect<H: InjectHandler>(
    sock_path: &Utf8Path,
    clock: &Arc<MockClock>,
    handler: H,
) { ... }
```

Attend's `InjectRouter` implements `InjectHandler<Msg = CaptureInject>`.
The `dispatch` method body is unchanged; it just moves to the trait
impl. `test_mode::connect()` becomes a thin wrapper:

```rust
pub fn connect() {
    let sock_path = cache_dir().join("test-inject.sock");
    let clock = CLOCK.get().expect("init not called").clone();
    let router = /* build InjectRouter from INJECT_ROUTER */;
    mock_clock::connect(&sock_path, &clock, router);
}
```

**Where does `connect` live?** It can live in `crates/mock-clock/`
alongside the clock (since it needs `MockClock::advance_and_settle()`),
or in a separate `crates/inject-client/` crate. The former is simpler
since the reader loop's only non-application dependency is the clock.
The `Handshake` and `Ack` types come from `crates/wire/`.

---

## Dependency order

```
1. Internal simplification          (no new crates)
       ↓
2. Wire protocol crate              (crates/wire/)
       ↓
3. MockClock extraction             (crates/mock-clock/)
       ↓
4. Generic harness + InjectHandler  (parameterize crates/test-harness/)
```

Items 2 and 3 are independent of each other (neither depends on the
other), but both must precede item 4 (which imports types from both).
Item 1 is a preparatory cleanup that makes item 4's diff smaller.

---

## What stays application-specific

These components are inherently attend-specific and are not candidates
for extraction:

- **Stubs** (`src/test_mode/stubs.rs`): `StubEditorSource`,
  `StubClipboardSource`, `StubExternalSource`, `StubAudioSource`.
  These implement attend's capture traits.

- **`InjectRouter`** (`src/test_mode.rs`): routes `CaptureInject`
  messages to the appropriate stub channels and shared state. Becomes
  the `InjectHandler` impl.

- **`CaptureConfig`** (`src/narrate/capture.rs`): bundles
  `Box<dyn Trait>` for each capture source, switches between production
  and test mode. Standard dependency injection, not harness logic.

- **Deferred connect** (`src/narrate/record.rs:1228-1233`): the daemon
  calls `connect()` after initialization so the harness knows "connected
  = ready." This timing decision is application-specific — the library
  provides `connect()`, the application decides when to call it.

- **E2e test helpers** (`tests/e2e.rs`): `activate()`, `collect()`,
  hook stdin JSON builders. Pure attend vocabulary.

- **`CaptureInject` variants** (`crates/wire/`): the specific message
  types (Speech, Silence, EditorState, ExternalSelection, Clipboard)
  are attend's domain. The harness is generic over them.

---

## Non-goals

- **Publish to crates.io.** All extracted crates remain `publish = false`
  workspace members. Publishing is a separate decision with its own API
  stability requirements.

- **Cross-process `InjectHandler`.** The `connect()` function runs
  inside the process under test. The harness is a separate process that
  only serializes messages. There is no shared Rust type between the
  harness process and the process under test at runtime — they
  communicate via JSON on a Unix socket. The shared types in
  `crates/wire/` are a compile-time convenience, not a runtime coupling.

- **Replace the deferred-connect pattern.** As analyzed in the preceding
  conversation: moving daemon connect to the top of `main()` is unsound
  because `advance_and_settle()` with zero sleeping threads returns
  immediately, sending vacuous ACKs during daemon initialization. The
  deferred connect is the simplest correct readiness signal and should
  be preserved.

---

## Gate

All existing tests (504 unit + 5 e2e) pass after each item. No
behavioral changes — the gate is purely: `cargo fmt --check && cargo
clippy && cargo nextest run`.
