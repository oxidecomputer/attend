# Socket-Based Daemon Redesign

Status: **Draft** (2026-02-26)

Supersedes: Phase 18 (Persistent Daemon)

## Current architecture

This section describes how things work today, for context. If you're
already familiar with the codebase, skip to [Motivation](#motivation).

### What attend does

`attend` is a voice narration tool: the user speaks while working, and
their speech is transcribed and delivered to an AI coding agent (Claude
Code) as context. The user can also capture browser selections, shell
commands, editor state, and clipboard content alongside speech.

### Key source files

| Area | Files | Purpose |
|------|-------|---------|
| Daemon lifecycle | `src/narrate/record.rs` | Recording state machine, sentinel polling, spawn/idle/shutdown |
| Audio capture | `src/narrate/audio.rs` | cpal microphone input, sample accumulation |
| Transcription | `src/narrate/transcribe/` | Parakeet and Whisper engines, model download |
| Capture threads | `src/narrate/capture.rs` | Coordinates editor, diff, ext, clipboard threads |
| Editor capture | `src/narrate/editor_capture.rs` | Polls editor for files/cursors, dwell filtering |
| Diff capture | `src/narrate/diff_capture.rs` | Watches file mtimes for content changes |
| External capture | `src/narrate/ext_capture.rs` | macOS accessibility API for selections in other apps |
| Clipboard capture | `src/narrate/clipboard_capture.rs` | Polls system clipboard for text/image changes |
| Event merge | `src/narrate/merge.rs` | Combines all event streams into a narration |
| Narration rendering | `src/view/` | Renders events as markdown for the agent |
| Hook layer | `src/hook.rs`, `src/hook/` | Claude Code hook integration (PreToolUse, PostToolUse) |
| Listener | `src/narrate/receive/listen.rs` | Background process that blocks until narration is ready |
| Browser bridge | `src/cli/browser_bridge.rs` | Native messaging host for browser extension |
| Shell hooks | `src/cli/shell_hook.rs` | Captures shell commands (fish/zsh preexec/postexec) |
| CLI | `src/cli/narrate.rs` | CLI command dispatch (toggle, pause, yank, status) |
| State/paths | `src/state.rs` | Cache dir, session IDs, listening state |
| Path constants | `src/narrate.rs` | All filesystem paths (sentinel, staging, pending, archive) |
| Chime | `src/narrate/chime.rs` | Audio feedback on start/stop |

### Daemon state machine

```
                    ┌─────────┐
                    │  idle   │◄──── daemon spawned, no recording yet
                    └────┬────┘
                         │ toggle
                         ▼
                    ┌──────────┐
              ┌────►│recording │◄────┐
              │     └────┬─────┘     │
              │          │ pause     │ resume (toggle while paused)
              │          ▼           │
              │     ┌──────────┐     │
              │     │  paused  │─────┘
              │     └──────────┘
              │
              │ toggle (while recording) or flush
              ▼
        ┌────────────┐
        │ finalizing │──→ transcribe → write narration → idle
        └────────────┘
```

The daemon also supports `yank` (finalize + copy to clipboard instead of
writing to pending) and `flush` (finalize + resume recording).

### How narration reaches the agent

The full narration protocol — what the agent sees, how it should respond,
content trust rules, and lifecycle edge cases — is documented in
[`src/agent/messages/narration_protocol.md`](../src/agent/messages/narration_protocol.md).
That file is injected into the agent's context when `/attend` activates
narration. The key architectural points are below.

This is a multi-process dance driven by Claude Code's hook system:

1. **`/attend` slash command**: The user types `/attend` in Claude Code.
   The `user-prompt-submit` hook runs `attend hook user-prompt`, which
   detects the `/attend` prompt and writes the session ID to
   `~/.cache/attend/hooks/listening`. This is the "activation" step.
   (Note: `session-start` is a separate hook that fires on session
   creation — it handles initial setup, not `/attend` activation.)

2. **`attend listen`**: Claude Code runs this as a background task. It
   holds an exclusive lock (`hooks/receive.lock`) and polls
   `narration/pending/<session_id>/` every 500ms. When files appear, it
   exits silently. Its exit is the signal to Claude Code that narration
   is available.

3. **`attend hook pre-tool-use`**: On every tool use, Claude Code calls
   this hook. If pending narration exists, the hook reads the JSON files,
   renders them as markdown, and prints them to stdout. Claude Code
   injects this output into the agent's conversation. The hook then
   archives the pending files and restarts `attend listen` as a new
   background task.

4. **Session theft**: If the user types `/attend` in a different Claude
   Code session, the `listening` file is overwritten with the new session
   ID. The old `attend listen` detects this on its next poll and exits.

The critical subtlety: `attend listen` is a **signal flare, not a data
channel**. Its task output is always empty. It exists solely so that its
exit triggers a `<task-notification>` in the agent, which prompts the
agent to run `attend listen` again — and it's the PreToolUse hook on
*that* restart call where narration actually gets delivered. The protocol
doc describes this as the "core loop."

### Current IPC: sentinel files and staging directories

The daemon is controlled via zero-byte "sentinel" files in
`~/.cache/attend/daemon/`:

- `stop` — CLI writes, daemon polls at 100ms, flushes and enters idle
- `pause` — daemon writes when entering idle; CLI deletes to resume
- `flush` — CLI writes, daemon flushes without stopping
- `yank` — CLI writes, daemon finalizes to `yanked/` instead of `pending/`

External events (browser selections, shell commands) are staged as JSON
files in `~/.cache/attend/staging/{browser,shell}/<session_id>/`. The
daemon collects these on flush/stop and merges them with other events.

A PID-based lock file (`daemon/lock`) provides exclusive instance
detection. A separate lock (`hooks/receive.lock`) prevents duplicate
listeners.

### Event types

All captured data flows through a unified `Event` enum (defined in
`src/narrate/merge.rs`):

- `Words` — transcribed speech with timestamps
- `EditorSnapshot` — open files, cursors, selections
- `FileDiff` — old/new content of changed files
- `ExternalSelection` — text selected in other apps (accessibility)
- `BrowserSelection` — text selected in browser (native messaging)
- `ShellCommand` — command text, exit status, duration
- `ClipboardSelection` — clipboard text or image path

The merge pipeline combines events from all sources, deduplicates,
orders by timestamp, and produces a single narration JSON array.

**Filtering is deferred to delivery time.** The daemon writes all events
with absolute paths, unfiltered. When the hook process delivers narration,
it filters to the agent's working directory (and `include_dirs`), redacts
out-of-scope events as `✂` markers, and relativizes paths. This means the
archive contains the full unfiltered narration, while the agent only sees
what's in scope. The same narration can be delivered to agents in different
working directories with different views.

---

## Motivation

The current daemon uses sentinel files (zero-byte files polled at 100ms) for
control, staging directories for external event ingestion, and `pending/` JSON
files for narration delivery. This works but has three problems:

1. **TCC permission inheritance**: On macOS, the daemon inherits its
   "responsible process" from whichever app spawned it (Zed, iTerm2, Shortcuts).
   The user must grant microphone access to every app they trigger narration
   from. If `launchd` spawns the daemon instead, TCC attributes the mic
   permission to `attend` itself — grant once, works everywhere.

2. **Polling overhead and latency**: Sentinel files are polled at 100ms. The
   browser bridge and shell hooks write to staging directories that aren't
   collected until flush/stop. A socket gives instant command delivery and
   real-time event ingestion.

3. **Filesystem sprawl**: Seven cache subdirectories (daemon/, staging/browser/,
   staging/shell/, staging/clipboard/, narration/pending/, narration/yanked/,
   hooks/) with sentinel files, lock files, staging JSON, and session markers.
   A socket collapses most of this into in-memory state.

## Design

### The daemon as central hub

```
                    ┌──────────────────────────────────────────────┐
                    │              attend daemon                   │
                    │                                              │
                    │  In-process capture threads:                 │
                    │    audio ────┐                               │
                    │    editor ───┤                               │
                    │    diff ─────┤  in-memory                    │
                    │    ext ──────┤  event buffer ──→ transcribe │
                    │    clipboard ┘       ↑                       │
                    │                      │                       │
                    │  ┌───────────────────┴──────────────┐        │
                    │  │    socket: control.sock          │        │
                    │  └───────────────────┬──────────────┘        │
                    │                      │                       │
                    └──────────────────────┼───────────────────────┘
                                           │
         ┌─────────────────────────────────┼──────────────────────┐
         │              │                  │           │          │
      CLI tool    browser bridge      shell hook   listener   hook layer
      (toggle,    (sends selections)  (sends cmds) (Wait for  (Activate,
       pause,                                       Ok)       Deactivate,
       yank,                                                  Collect,
       status)                                                Status)
```

All communication flows through a single Unix domain socket. The daemon
accepts multiple concurrent connections. Each connection sends a typed
message identifying itself and its intent.

### What moves to the socket

| Current mechanism | Replacement |
|-------------------|-------------|
| `daemon/stop` sentinel | `Command::Toggle` message |
| `daemon/pause` sentinel | `Command::Pause` message |
| `daemon/flush` sentinel | `Command::Flush` message |
| `daemon/yank` sentinel | `Command::Yank` message |
| `daemon/lock` (PID file) | Socket bind exclusivity |
| `hooks/listening` (session file) | Daemon-resident state; queried via socket |
| `hooks/receive.lock` | At most one `Wait` connection from listener |
| `staging/browser/*.json` | `Command::BrowserSelection` sent over socket |
| `staging/shell/*.json` | `Command::ShellCommand` sent over socket |
| `narration/pending/*.json` | `Command::Collect` retrieves directly from daemon |
| Status queries (read various files) | `Command::Status` → `Response::Status` |

### What stays on the filesystem

| Item | Why |
|------|-----|
| Model cache (~1.2 GB ONNX/GGML) | Cold storage, downloaded once |
| `narration/archive/` | Persistent history across daemon restarts |
| Config files (TOML) | User-edited, hierarchical |
| `version.json` (install metadata) | Written by `attend install`, read at startup |
| Clipboard image staging | Large PNGs; reference by path in events, clean up on archive |

### Narration delivery without pending files

Today, the daemon writes JSON to `pending/<session_id>/`. Delivery is a
two-process dance: `attend listen` is a background process that blocks
(holding `receive.lock`) until pending files appear, then exits. The actual
reading and delivery happens in the `attend hook pre-tool-use` process, which
runs synchronously in the agent's context — it reads the pending files,
renders them as markdown, and injects them into the agent's conversation.

With sockets, the listener's role stays the same — it's a poke that exits
when narration is available — but the mechanism changes from filesystem
polling to a blocking socket read:

1. `attend listen` connects to the daemon socket and sends
   `Command::Wait`. It does not know or send a session ID.
2. The daemon holds this connection open.
3. When narration is ready (stop, flush, or silence-triggered segment), the
   daemon sends `Response::Ok` to the waiting connection.
4. `attend listen` receives `Ok` and exits, causing the agent framework
   to fire the next tool use.
5. The hook process (`attend hook pre-tool-use`) connects to the daemon
   and sends `Command::Collect { session_id }` to retrieve the narration
   content directly — no filesystem intermediary. The hook knows the
   session ID because the agent framework passes it.

Session-theft detection is handled by the daemon: when a new session
activates (via `/attend`), the daemon closes any existing `Wait`
connection, causing the old listener to exit. Duplicate listener
prevention is enforced by the daemon allowing at most one `Wait`
connection at a time.

The daemon buffers finalized narration in memory until a `Collect` retrieves
it. The daemon stays resident (never exits from idle — it only unloads the
model), so buffered narration is only lost on crash.

This preserves the current two-process delivery model (listener pokes,
hook delivers) while eliminating filesystem polling.

### Yank without staging

Today, yank writes to `yanked/`, the parent CLI reads it back and copies to
clipboard. With sockets, the daemon handles everything:

1. CLI sends `Command::Yank`.
2. Daemon finalizes, transcribes, copies to clipboard directly (via
   `arboard`), and responds with `Response::Ok`.

No filesystem round-trip, no `yanked/` directory.

### Edge-case responses

- **`Collect` when nothing is buffered**: responds with
  `Narration { events: [] }` (empty array, not an error). The hook
  renders nothing and the agent sees no narration.
- **`Wait` when narration is already buffered**: responds with `Ok`
  immediately (no blocking). The listener exits, the hook collects.
- **`Toggle` when daemon has no active session**: starts recording
  without a session. Narration is buffered until a session activates
  and `Collect` is called.
- **`Yank` when not recording**: no-op, responds with `Ok`.

---

## Resolved decisions

1. **Clipboard images stay on disk.** Claude needs to read them by path.
   Image staging files remain in the filesystem; events reference them by
   path. Only text/metadata flows over the socket.

2. **Single listener only.** No multi-agent support. One session, one
   listener. `attend listen` does not activate a session — the `/attend`
   hook must be explicitly run first. Session stealing by running
   `attend listen` is not permitted.

3. **Cross-platform socket activation.** Use `service-binding` crate for
   both macOS (launchd) and Linux (systemd). Same daemon code path on both
   platforms, reducing variance. Service definitions are auto-managed on
   both platforms (no manual install step).

4. **Session state moves into the daemon.** The `sessions/` marker files
   (`displaced/`, `activated/`, `cache/`) are replaced by daemon-resident
   state. The hook process queries the daemon via socket instead of reading
   files. This means the daemon must stay resident (see below).

5. **Daemon stays resident, unloads model.** The daemon does not exit after
   idle timeout. It stays alive to hold session state and accept connections.
   After a dormancy period (configurable, default 5m), it unloads the
   transcription model to reclaim RAM (~1.2 GB for Parakeet, ~466 MB for
   Whisper). Both models are fully heap-allocated (not mmap'd), so the OS
   cannot reclaim them without swap — explicit unload is necessary for
   predictable memory behavior, especially on 8 GB machines.

   Unloading is straightforward: the transcriber is wrapped in an `Option`
   and set to `None` after dormancy. The underlying C libraries
   (`onnxruntime`, `whisper.cpp`) free their allocations via `Drop`. The
   model is re-loaded (~2-3s) on the next recording start. Socket, session
   state, and capture thread infrastructure remain live.

6. **Daemon archives before delivering.** On `Collect`, the daemon writes
   the narration to `archive/` first, then sends it to the hook. If the
   hook crashes after receiving, the narration is already persisted. No
   data loss window.

7. **Version field on every request.** Every `Request` struct carries a
   `version` field: the **git commit hash** baked into the binary at build
   time (via `build.rs`). The daemon checks it before processing; on
   mismatch it stops accepting connections, responds with `Error`, and shuts
   down. The service manager respawns it from the current binary. The
   client retries once.

   This also means `version.json` (install metadata) should use the commit
   hash instead of the cargo semver version, so all version checks are
   consistent.

   No separate handshake step — version checking is just part of every
   request. Single round-trip, no overhead.

---

## Protocol

### Framing

None. Each connection is a single round-trip: one request JSON object,
one response JSON object. `serde_json::to_writer()` /
`serde_json::from_reader()` directly on the `UnixStream`. No length
prefix, no newline delimiters, no framing code. Debuggable with `socat`.

### Serialization

`serde_json` in compact mode. Rationale:

- No cross-version compatibility needed (CLI and daemon are the same binary).
- JSON is inspectable with `socat` / `jq` during development.
- The messages are small (commands are tens of bytes; narration events are
  at most a few KB). Serialization speed is not a bottleneck.
- `serde_json` is already a dependency.

If profiling later shows serialization overhead matters (unlikely for control
plane; conceivable for high-frequency narration streaming), `postcard` is a
drop-in replacement (same serde derives, binary format, ~30% smaller messages).

### Message types

```rust
/// Client → Daemon
///
/// Every request carries the client's commit hash. The daemon checks
/// it before processing; on mismatch it responds with Error, stops
/// accepting new connections, and shuts down. The client retries once
/// (the service manager respawns the daemon from the current binary).
#[derive(Serialize, Deserialize)]
struct Request {
    version: String,  // git commit hash
    command: Command,
}

#[derive(Serialize, Deserialize)]
enum Command {
    // Recording control (Toggle and Pause are idempotent toggles —
    // the daemon decides the transition based on current state)
    Toggle,
    Pause,
    Flush,
    Yank,

    // External events
    BrowserSelection {
        url: String,
        title: String,
        html: String,
        plain_text: Option<String>,
    },
    ShellCommand {
        shell: String,
        command: String,
        cwd: String,
        exit_status: Option<i32>,
        duration_secs: Option<f64>,
    },

    // Session management (from hook layer)
    ActivateSession { session_id: String },
    DeactivateSession { session_id: String },

    // Listener: block until narration is ready (no session ID needed)
    Wait,

    // Hook: collect pending narration for delivery
    Collect { session_id: String },

    // Queries
    Status,
}

/// Daemon → Client
#[derive(Serialize, Deserialize)]
enum Response {
    // Success. The CLI interprets this based on what it sent.
    Ok,

    // Failure (including version mismatch).
    Error { message: String },

    // Collected narration content (response to Collect)
    Narration { events: Vec<Event> },

    // Full status report (response to Status)
    Status { /* fields from current status.rs */ },
}
```

### Version handshake

Every request carries a `version` field (the client's commit hash). The
daemon checks it before processing the command:

- Match: process the command normally.
- Mismatch: stop accepting new connections, ONLY THEN respond with `Error`, and
  shut down. The service manager respawns the daemon from the current
  binary. The client retries once.

The commit hash is baked in at build time via `build.rs` writing to
`env!("ATTEND_COMMIT_HASH")`. For dev builds without a clean commit,
use the full `git describe --always --dirty` output.

No special handshake step — version checking is just part of every
request.

### Connection patterns

| Client | Pattern | Lifecycle |
|--------|---------|-----------|
| `attend narrate toggle` | Connect → send `Toggle` → receive `Ok` → disconnect | Ephemeral |
| `attend narrate pause` | Connect → send `Pause` → receive `Ok` → disconnect | Ephemeral |
| `attend narrate yank` | Connect → send `Yank` → receive `Ok` (daemon copies to clipboard) → disconnect | Ephemeral |
| `attend narrate status` | Connect → send `Status` → receive `Status` → disconnect | Ephemeral |
| `attend browser-bridge` | Connect → send `BrowserSelection` → receive `Ok` → disconnect | Ephemeral |
| `attend shell-hook` | Connect → send `ShellCommand` → receive `Ok` → disconnect | Ephemeral |
| `attend listen` | Connect → send `Wait` → block until `Ok` → disconnect | Long-lived (blocking) |
| `attend hook pre-tool-use` | Connect → send `Collect` → receive `Narration` → disconnect | Ephemeral |
| `/attend` hook | Connect → send `ActivateSession` → receive `Ok` → disconnect | Ephemeral |
| `/unattend` hook | Connect → send `DeactivateSession` → receive `Ok` → disconnect | Ephemeral |

Every connection is a single round-trip: one request, one response. For
`Wait`, the response is simply delayed until narration is ready.

---

## Daemon lifecycle

### Startup

On both platforms, the service manager (launchd on macOS, systemd on
Linux) is auto-managed. The `service-binding` crate provides a unified
interface for socket activation across both.

1. The service definition (plist or systemd unit) is auto-installed (see
   service management section).
2. The service manager creates `control.sock` and listens on it.
3. First client connects → service manager spawns `attend narrate _daemon`.
4. Daemon calls `service-binding` to receive the activated socket fd.
5. Converts to `UnixListener` and begins accepting connections.

**Fallback (no service manager)**: If socket activation fails (e.g., no
systemd on a minimal Linux), the CLI spawns the daemon directly with
`process_group(0)` + detached stdio. The daemon creates the socket itself.

### Socket path

`$CACHE_DIR/daemon/control.sock`

On macOS: `~/Library/Caches/attend/daemon/control.sock`
On Linux: `$XDG_CACHE_HOME/attend/daemon/control.sock` (typically
`~/.cache/attend/daemon/control.sock`)

### Idle and model unloading

The daemon stays resident indefinitely. It does not exit on idle — it holds
session state and accepts connections at all times. After a dormancy period
(configurable, default 5m) with no active recording, the daemon unloads the
transcription model to reclaim RAM. The model is re-loaded on the next
recording start.

The daemon only exits on:
- Version mismatch (client has a newer commit hash)
- Explicit `attend uninstall`
- Crash (service manager restarts it)

### Exclusive instance

Socket bind is itself an exclusive lock — if the socket path exists and is
bound, `bind()` fails with `EADDRINUSE`. This replaces the PID lock file.

Stale socket detection (fallback path only — service managers handle
restarts automatically): if `connect()` to an existing socket fails with
`ECONNREFUSED`, the daemon has crashed without cleaning up. The CLI removes
the stale socket and spawns a new daemon.

---

## Service management (cross-platform)

### macOS: LaunchAgent plist

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.attend.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>ATTEND_BIN_PATH</string>
        <string>narrate</string>
        <string>_daemon</string>
    </array>
    <key>Sockets</key>
    <dict>
        <key>attend</key>
        <dict>
            <key>SockFamily</key>
            <string>Unix</string>
            <key>SockPathName</key>
            <string>SOCKET_PATH</string>
        </dict>
    </dict>
</dict>
</plist>
```

`ATTEND_BIN_PATH` and `SOCKET_PATH` are templated at install time.

### Auto-managed (no separate install step)

On macOS, launchd management is the only mode of operation — there is no
`--daemon` flag or opt-in. Any CLI command that needs the daemon (toggle,
pause, yank, status) ensures the plist is installed and current before
connecting to the socket:

1. Read the installed plist (if any) from `~/Library/LaunchAgents/`.
2. Compare the `ProgramArguments` path against the current `attend` binary
   (`std::env::current_exe()`).
3. If missing or stale (binary path changed after upgrade/reinstall):
   - Write the new plist.
   - `launchctl bootout` the old service (if loaded).
   - `launchctl bootstrap` the new one.
4. Connect to the socket. launchd spawns the daemon automatically on
   first connect — there is no explicit "start daemon" step.

This is silent and automatic. `attend uninstall` removes the plist and
runs `launchctl bootout` as part of full cleanup.

### Linux: systemd user service

Two unit files: a `.socket` (holds the socket) and a `.service` (runs the
daemon when activated):

```ini
# ~/.config/systemd/user/attend-daemon.socket
[Unit]
Description=attend daemon socket

[Socket]
ListenStream=SOCKET_PATH
SocketMode=0600

[Install]
WantedBy=sockets.target
```

```ini
# ~/.config/systemd/user/attend-daemon.service
[Unit]
Description=attend narration daemon
Requires=attend-daemon.socket

[Service]
Type=simple
ExecStart=ATTEND_BIN_PATH narrate _daemon
```

`ATTEND_BIN_PATH` and `SOCKET_PATH` are templated at install time (same
as the macOS plist).

Same auto-management pattern. Any CLI command that needs the daemon ensures
the systemd user service is installed and current:

1. Check `~/.config/systemd/user/attend-daemon.service` and
   `attend-daemon.socket`.
2. If missing or stale: write units, `systemctl --user daemon-reload`,
   `systemctl --user enable --now attend-daemon.socket`.
3. Connect to the socket. systemd spawns the daemon on first connect.

`attend uninstall` disables and removes the units.

### TCC effect

Because `launchd` spawns the daemon, TCC attributes microphone access and
accessibility permissions to the `attend` binary. The user grants access once
(on first recording start), and it works regardless of which app triggered the
hotkey.

---

## Crate choices

| Purpose | Crate | Notes |
|---------|-------|-------|
| Socket listener | `std::os::unix::net::UnixListener` | No async runtime needed |
| Framing | None | One JSON object per connection; `serde_json` reads/writes directly |
| Serialization | `serde_json` | Already a dependency; debuggable; swap to `postcard` later if needed |
| Socket activation | `service-binding` | Cross-platform: launchd (macOS) and systemd (Linux) |
| Service unit mgmt | Hand-rolled (template + write) | Plist and systemd units are static with templated paths |

### No async runtime

The daemon is CPU-bound (transcription) and I/O-bound on platform APIs (cpal,
accessibility) that are inherently synchronous. The control plane has at most
a handful of concurrent connections — usually just one. There is no workload
here that benefits from async.

A dedicated acceptor thread calls `listener.accept()` in a loop. Ephemeral
connections (command → response) are handled inline on the acceptor thread.
The one long-lived connection (`Wait`) gets its own thread that blocks on a
channel receiver until the daemon signals readiness. No tokio, no futures,
no async runtime overhead.

---

## Migration path

Red-green is the north star. The oracle test suite must pass against the
current implementation before any functional changes begin. Each subsequent
phase is: make the change, get back to green.

### Phase 0: Test infrastructure and oracle suite

**No functional changes.** This phase only adds testability and tests.

1. **Trait extraction for capture sources.** Audio, editor, diff, clipboard
   capture need trait-based injection (ext capture already has
   `ExternalSource`). Introduce a `CaptureConfig` struct that bundles
   factory functions for each source. Production code uses real
   implementations; test mode substitutes stubs.

2. **Trait extraction for transcription.** The transcription model needs a
   trait so the stub can accept injected text from the test harness.

3. **Clock trait and `Instant` elimination.** Replace all `Instant::now()`
   and `Utc::now()` with a `Clock` trait (`now() → DateTime<Utc>`).
   Replace `thread::sleep()` with `clock.sleep()`. Production uses real
   time; test mode uses a mock clock advanced only by `AdvanceTime`.
   This is a prerequisite for deterministic time in tests.

4. **`ATTEND_TEST_MODE` and `ATTEND_CACHE_DIR` env vars.** `ATTEND_TEST_MODE=1`
   swaps in stub capture sources and opens the `test-inject.sock` side
   channel. `ATTEND_CACHE_DIR` controls the cache directory: set to a
   path to use that path, or set to empty (`""`) to auto-create a random
   temp directory (useful for parallel test runs). No behavioral change
   to production code paths.

5. **End-to-end test harness.** `TestHarness` struct that spawns a real
   daemon in test mode, drives it via real CLI subprocesses, and asserts
   on outputs. All IPC is real (whatever mechanism the binary under test
   uses).

6. **Declarative oracle.** State-machine invariants as proptest
   postconditions. Must pass green against the current implementation.

7. **Proptest action-sequence fuzzer.** Random interleavings of
   toggle/pause/yank/browser-event/shell-event/wait/collect/status
   plus injections (transcript, editor, ext, clipboard, time). Asserts
   invariants after each sequence. Must pass green.

**Dependency order within Phase 0:**

```
1. Trait extraction (capture)  ──┐
2. Trait extraction (transcribe) ┤
3. Clock trait                   ├→ 4. Env vars + test-inject.sock → 5. Harness → 6. Oracle → 7. Fuzzer
```

Items 1-3 are independent of each other. Item 4 (env vars + inject socket)
depends on the traits existing. The inject socket is a minor structural
change to the daemon: it adds a listener thread for `test-inject.sock`
that routes injections to the stub trait impls via channels. This is the
one change in Phase 0 that isn't pure refactoring.

**Gate**: the full oracle suite passes reliably before proceeding.

### Phase A: Socket control plane

Replace sentinel files with socket-based commands. The daemon listens on a
Unix domain socket. CLI commands (`toggle`, `pause`, `flush`, `yank`)
connect and send typed messages instead of writing sentinel files.

- Lock file → socket bind exclusivity
- Sentinel polling loop → socket accept loop (blocking or select-based)
- `attend narrate status` → queries daemon over socket
- Version handshake on every connection (commit hash)
- `version.json` switches from cargo semver to commit hash
- Service manager auto-management on macOS (launchd) and Linux (systemd)
- Staging directories for browser/shell remain (Phase B)
- `pending/` files remain (Phase C)

**Gate**: both oracle suites pass green.

### Phase B: External event ingestion

Browser bridge and shell hooks send events directly to the daemon socket
instead of writing to staging directories.

- `staging/browser/` eliminated
- `staging/shell/` eliminated
- Events are merged in real-time (no deferred collection on flush)
- Timestamps come from the event itself, not from file mtime
- With a service manager (launchd or systemd), the socket is always
  available; connecting wakes the daemon. On the fallback path (no service
  manager), if the daemon isn't running, the event is dropped — there's
  no active narration session to deliver it to.

**Gate**: both oracle suites pass green.

### Phase C: Narration delivery and session state over socket

`attend listen` blocks on a `Wait` command instead of polling the filesystem.
The hook process collects narration via `Collect` instead of reading
`pending/` files. Session state moves into the daemon.

- `hooks/receive.lock` → at most one `Wait` connection
- `attend listen` filesystem poll → `Wait` on socket, exits on `Ok`
- Hook `collect_pending()` from files → `Collect` over socket
- Daemon buffers finalized narration in memory until `Collect`
- Daemon archives narration before delivering to hook
- `sessions/` markers replaced by daemon-resident state
- Daemon stays resident, unloads model after dormancy period

**Gate**: both oracle suites pass green.

### Phase D: Cleanup

- Remove dead code for sentinel file handling, staging directory management,
  lock file creation
- Remove now-unused cache subdirectories (`staging/`, `narration/pending/`,
  `narration/yanked/`, `sessions/`)
- Update `docs/setup.md` and troubleshooting
- Update `attend narrate status` output (socket path, connection state)

**Gate**: both oracle suites pass green. Final audit.

Note: [`narration_protocol.md`](../src/agent/messages/narration_protocol.md)
should **not** need changes. The agent-facing behavior is identical:
`attend listen` still exits to signal readiness, the PreToolUse hook
still delivers narration on stdout. The socket is entirely below the
agent's abstraction boundary.

---

## End-to-end testing

The migration must not change observable behavior. We want a test suite that
can validate both the current (sentinel-file) and new (socket) implementations
against the same expectations, and that can fuzz arbitrary action sequences to
surface races and edge cases.

### Test mode activation

An environment variable `ATTEND_TEST_MODE=1` triggers test configuration:

- **Audio and transcription**: entirely stubbed. No cpal, no sound card,
  no model loading, no network. `Inject::Speech { text, duration_ms }`
  combines what was said with how long it took — the stub transcriber
  returns the injected text directly, bypassing the real model. Chime
  playback is a no-op. This is essential for fuzzing at thousands of
  times realtime.
- **Editor capture**: replaced with a stub that returns scripted file lists
  and cursor positions, driven by a fixture file or env var.
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

The capture sources are already partially behind traits (`ExternalSource`).
The others (audio, editor, clipboard) need trait extraction or a
`CaptureConfig` struct that bundles factory functions for each source. The
env var selects the stub config; production code is the default.

### Two oracle models

#### Oracle 1: Differential binary

A standalone binary (`attend-oracle-diff`) that takes two `attend` binary
paths and fuzzes them against each other:

```
attend-oracle-diff --binary-a ./target/release/attend-old \
                   --binary-b ./target/release/attend-new
```

Internally, this binary uses proptest to generate and shrink action
sequences. On failure, proptest's shrinking produces the minimal
reproducing sequence — not a wall of 50 random actions, but the 3-4
that actually trigger the divergence.

For each test case:
1. proptest generates a random action sequence (with injections).
2. Spin up two isolated environments (separate `ATTEND_CACHE_DIR`, each
   set to a fresh temp dir).
3. Run the same action sequence against both binaries via real CLI
   subprocesses, with `ATTEND_TEST_MODE=1`.
4. Compare outputs: delivered narrations, status reports, yank results,
   daemon exit behavior.
5. On mismatch: proptest shrinks the sequence and reports the minimal
   failing case.

This is a separate binary (in `src/bin/` or a workspace member), not a
`#[test]`. It has no dependency on `attend` internals — it only shells out
to the two binaries. This means it works across any two commits: build the
old commit, build the new one, point the oracle at both.

**Self-diff as smoke test**: The oracle's own validation is running the
same binary against itself. This must produce all-green. If it doesn't,
the oracle has nondeterminism bugs. But passing self-diff is necessary,
not sufficient — the assertions must be tight enough to catch real
divergences. Key differential assertions:

- Collected narration contains exactly the same events in the same order
- Each injected transcript string appears verbatim in the output
- Each injected browser/shell/editor/ext event appears in the output
- Status report fields match (recording, paused, engine, pending count)
- Yank produces identical clipboard content
- Archive directory contains the same files with the same content
- Daemon exit behavior matches (alive vs exited, exit code)

If these all pass when diffing a binary against itself, and they're
tight enough that swapping in a broken binary would fail, the oracle is
sound.

The typical workflow during migration:

```bash
# Build baseline in a worktree (one-time setup)
git worktree add ../attend-baseline main
cargo build --release --manifest-path ../attend-baseline/Cargo.toml

# Build current work
cargo build --release

# Diff them
cargo run --bin attend-oracle-diff -- \
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

#### Oracle 2: Declarative specification

A state-machine specification that describes expected behavior independently
of either implementation. For example:

- "After Toggle (start) + Toggle (stop), at most one narration should be
  collectible, and if words were spoken, exactly one will be."
- "After Toggle (start) + Pause + Pause (resume) + Toggle (stop), narration
  includes events from both recording periods."
- "After Toggle (start) + BrowserEvent + Toggle (stop) + Collect, the
  delivered narration contains the browser event."
- "After Yank, clipboard is non-empty."
- "Start while already recording is a no-op."
- "Stop while idle is a no-op."
- "Wait without pending narrations blocks until the next flush."

These are invariants, not exact output comparisons. They can be expressed as
proptest postconditions on the action sequence. This oracle survives
implementation changes (e.g., if we later change merge ordering or timestamp
precision) where the differential oracle would break.

The declarative oracle is also a standalone binary (`attend-oracle-spec`)
that takes a single `attend` binary path. It uses proptest internally for
generation and shrinking — a failing invariant produces the minimal action
sequence that violates it. It's the durable asset; the differential oracle
is the migration safety net.

### Injection: how to feed events into the daemon

The daemon's capture sources (audio, editor, ext, clipboard) are in-process
threads. The test harness can't inject events via the main CLI — it needs a
side channel into the daemon.

In test mode, the daemon opens a second socket: `test-inject.sock` (in the
same cache dir). The harness sends injection commands over this socket:

```rust
/// Harness → Daemon (test-inject socket only)
#[derive(Serialize, Deserialize)]
enum Inject {
    /// Inject speech: what was said and how long it took.
    Speech { text: String, duration_ms: u64 },
    /// Inject a period of silence.
    Silence { duration_ms: u64 },
    /// Stub editor capture returns this state on next poll.
    EditorState { files: Vec<FileEntry> },
    /// Stub ext capture returns this selection on next poll.
    ExternalSelection { app: String, text: String },
    /// Stub clipboard capture emits this content on next poll.
    Clipboard { text: String },
    /// Advance the daemon's mock clock by this duration. All time-dependent
    /// behavior (idle timeout, model unload, dwell timers, selection merge
    /// windows, silence segmentation) sees the jump immediately.
    AdvanceTime { duration_ms: u64 },
}
```

This is dynamic — the harness can inject events at any point in the action
sequence, interleaved with CLI commands. The inject socket only exists
when `ATTEND_TEST_MODE=1`; production builds never open it.

Time manipulation via `AdvanceTime` requires a clock trait behind all time
sources in the daemon. Production uses real `Utc::now()` /
`thread::sleep()`; test mode uses a mock clock that only advances when
`AdvanceTime` is injected. This eliminates real wall-clock delays from
tests entirely — a sequence that exercises idle timeout and model unloading
runs in milliseconds.

The clock trait is a single operation:
- `now()` → `DateTime<Utc>`

All timeouts and durations are computed as differences between two `now()`
calls. `Instant::now()` is eliminated entirely — it's opaque, can't be
serialized or mocked by a known amount, and everything that leaves the
daemon (narration, archive, status) uses UTC anyway. This is a cleanup
worth doing independent of the socket migration.

`thread::sleep()` is replaced by `clock.sleep(duration)`:
- Production: real `thread::sleep()`, wall clock advances naturally.
- Test mode: yields immediately without advancing the clock. Multiple
  threads sharing a clock sleep concurrently in production; advancing
  the clock from one thread's sleep would double-count time.

Time advances only from one source: `AdvanceTime` injected by the harness.
The harness is in full control — it injects events, advances time by known
amounts, and checks results. The daemon never autonomously decides "5
minutes have passed"; the harness says so.

This means daemon poll loops spin in test mode (sleep returns immediately,
clock doesn't move). This is fine: capture threads block on stub channels
waiting for injected events, not on sleep. The main loop blocks on
`accept()` (socket) or checks sentinels (current impl) each iteration,
which is a no-op when no commands are pending. CPU spin during tests is
acceptable for correctness.

The mock clock is an `Arc<Mutex<DateTime<Utc>>>` that `AdvanceTime` bumps
forward. Simple, deterministic, no thread-interaction hazards.

Browser and shell events don't need injection — they're already external
CLI commands (`attend browser-bridge`, `attend shell-hook`) that the
harness invokes directly.

### Observation: what the harness can check

Both oracles are black-box: they only observe externally visible behavior.

| Observable | How | Checked by |
|-----------|-----|-----------|
| CLI stdout/stderr | Capture `Command` output | Both oracles |
| CLI exit code | `Command` status | Both oracles |
| Collected narration content | `attend hook pre-tool-use` stdout | Both oracles |
| Narration contains injected transcript | String match on collected text | Both oracles |
| Narration contains injected events | Check for browser/shell/editor/ext events | Both oracles |
| Clipboard content (after yank) | Read from stub buffer (not real clipboard) | Both oracles |
| Archive files on disk | Read `archive/` in isolated cache dir | Both oracles |
| Daemon process liveness | Check PID / process status | Both oracles |
| Status report fields | Parse `attend narrate status` output | Both oracles |

The harness cannot observe daemon-internal state (in-memory buffers, thread
state, model load status). This is intentional — the oracle validates the
contract, not the implementation.

For the **differential oracle**, observations from both binaries are compared
field by field. For the **declarative oracle**, observations are checked
against invariants.

### Test harness

Both oracles share the same harness for driving a single `attend` binary:

```rust
/// E2E test fixture: starts a daemon in test mode with isolated cache dir,
/// provides helpers to invoke CLI commands and assert on outputs.
struct TestHarness {
    /// Path to the `attend` binary under test.
    binary: Utf8PathBuf,
    /// Isolated temp directory for all cache state.
    cache_dir: TempDir,
}

impl TestHarness {
    // --- CLI commands (real subprocess invocations) ---

    fn toggle(&self) -> Output { ... }
    fn pause(&self) -> Output { ... }
    fn yank(&self) -> Output { ... }
    fn status(&self) -> StatusReport { ... }
    fn browser_event(&self, url: &str, text: &str) -> Output { ... }
    fn shell_event(&self, cmd: &str, exit: i32) -> Output { ... }
    fn collect(&self, session_id: &str) -> Vec<Narration> { ... }
    fn activate_session(&self, session_id: &str) -> Output { ... }

    // --- Injections (via test-inject.sock) ---

    fn inject_speech(&self, text: &str, duration_ms: u64) { ... }
    fn inject_silence(&self, duration_ms: u64) { ... }
    fn inject_editor_state(&self, files: &[FileEntry]) { ... }
    fn inject_external_selection(&self, app: &str, text: &str) { ... }
    fn inject_clipboard(&self, text: &str) { ... }
    fn advance_time(&self, duration_ms: u64) { ... }
}
```

Each CLI invocation is a real `std::process::Command` call to `self.binary`,
with `ATTEND_TEST_MODE=1` and `ATTEND_CACHE_DIR=<temp>` set. This exercises
the full code path: argument parsing, IPC (whatever mechanism the binary
uses), daemon handling, response serialization.

The harness is agnostic to the IPC mechanism — it only cares about CLI
inputs and observable outputs. This is what makes the differential oracle
work across implementation boundaries.

**Daemon lifecycle per test case:** Each proptest case gets a fresh daemon.
The harness spawns the daemon at the start of the case and sends SIGTERM
(or, in the socket world, a shutdown command) at the end. To keep this
fast, the daemon in test mode should start up quickly (no model loading,
no real audio init). If startup latency is still a concern, the harness
could reuse a daemon across cases by sending a `Reset` inject command
that clears all state — but start with the simple approach first.

**How `collect()` works:** The harness always invokes the real
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
- `cwd`: the harness's isolated cache dir (or a subdirectory of it).
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
`assert_decision`, `assert_activation`). The e2e harness should parse
the hook's stdout/stderr back into these same types and reuse the
assertion vocabulary. This avoids reimplementing hook response parsing
and keeps the e2e invariants consistent with the unit-level hook tests.

The hook outputs structured text (guidance for the agent, narration
content in `<narration>` tags, blocking messages). The e2e harness
needs a parser that maps this output back to `Outcome` variants.
This parser should live in a shared crate or module accessible to both
the hook unit tests and the e2e oracle binaries.

### Proptest fuzzing

```rust
#[derive(Debug, Clone, Arbitrary)]
enum Action {
    // Session lifecycle
    ActivateSession { session_id: String },
    DeactivateSession { session_id: String },

    // Recording control
    Toggle,
    Pause,
    Flush,
    Yank,

    // External events (via CLI)
    BrowserEvent { url: String, text: String },
    ShellEvent { cmd: String, exit_status: i32 },

    // Delivery
    Wait,
    Collect { session_id: String },
    Status,

    // Injections (via test-inject.sock)
    AdvanceTime { duration_ms: u64 },
    InjectSpeech { text: String, duration_ms: u64 },
    InjectSilence { duration_ms: u64 },
    InjectEditorState { files: Vec<String> },
    InjectExternalSelection { app: String, text: String },
    InjectClipboard { text: String },
}
```

**Structured sequence generation.** Raw random interleaving produces
mostly nonsensical sequences (Collect before ActivateSession, Yank
while idle). The proptest strategy should generate *structured*
sequences with random perturbations:

1. Activate a session (random UUID, constant within the sequence).
2. Toggle (start recording).
3. Random interleaving of injections and external events.
4. Toggle (stop) or Yank.
5. Wait + Collect.
6. Optionally repeat 2-5.
7. Optionally DeactivateSession.

Each step can be randomly omitted or reordered to test error handling
(e.g., Collect without prior Toggle, double Toggle, Wait without
ActivateSession). The structured shape biases toward realistic
sequences while still exploring edge cases. Proptest shrinking will
strip away the random perturbations to find minimal failing cases.

**Two fuzzer strategies, both run by each oracle:**

- **Structured**: the biased strategy above. Finds bugs in the happy
  path and realistic edge cases.
- **Totally random**: `proptest::collection::vec(any::<Action>(), 1..50)`
  with no structure at all. Finds bugs in error handling, unexpected
  command ordering, and state machine robustness (double yank, collect
  before activate, pause while idle, etc.). Most sequences will be
  nonsensical — that's the point.

Used by both oracles:
- **Differential**: generate sequence, run against both binaries, diff.
- **Declarative**: generate sequence, run against one binary, assert
  invariants.

Invariants checked after each sequence:
- No panics in daemon (process alive or exited cleanly)
- No orphaned temp files outside expected dirs
- Delivered narrations contain all injected events (no loss)
- State is consistent (not recording + not paused after stop)
- Injected transcript text appears in collected narrations

### What this replaces vs complements

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

## Agent handoff

This project is too large for a single agent context window. Each phase
(and likely each numbered item within Phase 0) will be a separate agent
session. To enable clean handoff:

1. **This document is the source of truth.** The implementing agent should
   read this file first. It contains the full design, all resolved
   decisions, and the migration ordering.

2. **Phase status tracker.** Each phase has a gate (oracle suites pass
   green). Update the status at the top of this file and mark completed
   phases as done, with the commit hash where the gate was met.

3. **Per-phase notes.** When a phase is in progress, keep a short log of
   implementation decisions and deviations at the bottom of this file
   under a "## Implementation notes" heading. The next agent reads these
   to understand what was done and why.

4. **Commit frequently.** Each trait extraction, each stub, each oracle
   invariant should be its own commit. This gives the next agent a clear
   `git log` to understand progress.

5. **CLAUDE.md / memory.** Update the project memory file with the current
   phase and any gotchas discovered during implementation.

6. **Existing tests are a friend, not an obstacle.** All existing tests
   should be assumed correct in *intent*. They may need mild syntactic
   adaptation to work with the new architecture (e.g., a trait parameter
   added, a mock injected), but their invariants should never be relaxed.
   If a test fails after a change, the default assumption is that the
   change is wrong, not the test. If investigation reveals a genuine bug
   in the current implementation, fix it — but be cautious and discerning.
   Every test modification should be in the spirit of greater rigor,
   never relaxing invariants in the name of expediency. This applies
   equally to tests written during this project: once green, they are
   the new baseline.

## Open questions

None at this time. All major design decisions are resolved above.

---

## Appendix: `responsibility_spawnattrs_setdisclaim()`

This private macOS API could break the TCC responsible-process chain without
requiring a LaunchAgent. It still works on Sequoia 15.x and is used by LLDB,
Qt Creator, and Facebook's sado project. Ghostty evaluated and rejected it,
but their concern (shells as privilege escalation trampolines) doesn't apply
to us: our daemon is a bounded, known binary, not an arbitrary-code executor.

However, the service-manager approach is strictly better:
- Supported, public API
- Gives us on-demand activation for free
- Does not require `unsafe` `posix_spawn` calls
- The socket-based architecture is independently valuable
- Works cross-platform (systemd on Linux, not just macOS)

Documented here for posterity. Not planned for use.
