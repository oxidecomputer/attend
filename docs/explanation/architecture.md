# About attend's architecture

This page discusses the design patterns and decisions behind attend's
implementation. For a user-facing overview of the system, see
[how it works](how-it-works.md). For build instructions, see
[development](../extending/development.md).

## Architectural patterns

### Trait-based backends

Editors, agents, shells, and browsers each implement a trait on a zero-sized
struct. Backends are registered in a static slice and the CLI wires them up
automatically. This means adding a new integration is mechanical: implement the
trait, register the struct, and everything downstream (install, uninstall, hook,
narrate) discovers it automatically.

The zero-sized struct pattern avoids any runtime state in the backend objects
themselves. All state lives in the filesystem or in function arguments, which
makes backends trivially testable.

### Filesystem-based IPC

The CLI and daemon communicate through atomically written files rather than
persistent connections (sockets, pipes). The daemon writes a status file; the
CLI writes a command file; each side polls the other.

This is deliberately simple. A socket-based protocol would have lower latency
but would add complexity around connection management, reconnection, and error
handling. The filesystem approach is robust to crashes (stale files are
detected and cleaned up) and trivially debuggable (you can `cat` the status
file). The latency cost is small enough to be imperceptible in practice — a
few hundred milliseconds at most.

### Pure decision logic

The hook decision table (`hook/decision.rs`) is a pure function from session
state to action, with no side effects. The orchestrator collects state (session
ID, pending narration, listener status), passes it to the decision function,
and then executes the resulting action. This separation makes the decision
logic exhaustively testable without mocking any I/O.

### Time abstraction

All code uses a `Clock` trait. Production uses `RealClock`; tests use
`MockClock` with deterministic, explicitly-advanced time. This enables a kind
of "low-rent [Antithesis](https://antithesis.com/)": tests can drive the
entire system — including daemon startup, recording, transcription, and
delivery — through controlled time steps, without any real-time waits or
flaky sleeps.

## Test architecture

Tests are organized by scope:

- **Unit tests** live in `<module>/tests.rs` sibling files (not inline
  `mod tests` blocks). Each test has a doc comment stating the invariant.

- **Integration tests** live in `tests/`. The test harness
  (`crates/test-harness`) provides end-to-end testing by spawning real
  processes and driving them via CLI.

- **Property tests** use `proptest` for state-invariant testing. Regression
  files in `proptest-regressions/` are committed to ensure discovered failures
  reproduce deterministically.

### The test harness

The test harness (`crates/test-harness`) enables deterministic end-to-end
testing:

- Spawns real `attend` processes with `ATTEND_TEST_MODE=1`
- Injects events (audio, editor state, time) via a Unix socket protocol
- Uses `MockClock` across all processes for lockstep time advancement
- Settlement protocol ensures all threads complete their work before time
  advances
- No polling or real-time waits — condvar-based coordination throughout

The harness makes it possible to test scenarios that would otherwise be
inherently racy: "start recording, wait for the daemon to capture two editor
snapshots, stop recording, verify the narration contains both snapshots" — all
without any `sleep()` calls or timing assumptions.
