//! End-to-end test harness for attend.
//!
//! Spawns a real `attend` daemon in test mode, drives it via real CLI
//! subprocesses, injects events through the inject socket, and asserts on
//! observable outputs. All IPC uses whatever mechanism the binary under
//! test implements — the harness is black-box.
//!
//! # Architecture
//!
//! The harness is the **inject socket server**. It binds
//! `$ATTEND_CACHE_DIR/test-inject.sock` before spawning any processes.
//! Every process spawned with `ATTEND_TEST_MODE=1` connects to this
//! socket, sends a handshake, and reads injection messages.
//!
//! A **background accept thread** runs a blocking accept loop. Each new
//! connection is read for its handshake (PID + argv), then inserted into
//! shared state behind a `Mutex` + `Condvar`. The foreground test thread
//! waits on the condvar for specific PIDs or daemon connections — no
//! polling, no sleeps, no non-blocking accept.
//!
//! # Execution model: all processes are background
//!
//! Every CLI command, hook invocation, and listener is launched as a
//! background child. The harness never blocks waiting for a specific
//! process to exit. Instead, it advances mock time and checks all children
//! for exits via `try_wait()` after each tick settlement. Process exits
//! are captured as [`TraceEvent`] entries.
//!
//! # Time coordination
//!
//! All processes under test use a `MockClock` with condvar-gated sleep.
//! The harness advances time by broadcasting `AdvanceTime` messages and
//! waiting for ACK from each process before proceeding. This gives
//! lockstep execution across OS processes.

mod protocol;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};

pub use protocol::FileEntry;
use protocol::{CaptureInject, Handshake, Inject, TimeInject};

// ---------------------------------------------------------------------------
// Configuration constants
// ---------------------------------------------------------------------------

/// How long (wall-clock) to wait for a process to connect to the inject socket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum wall-clock time to wait for a process to exit during
/// `tick_until_exit`.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Mock time increment per tick (ms). Must be <= the smallest poll
/// interval in the codebase (50ms for sentinel polling).
const TICK_MS: u64 = 50;

/// Wall-clock timeout for reading an ACK from a process after
/// `AdvanceTime`. Normal ACKs arrive quickly (condvar-based settlement
/// in `wait_for_waiters` blocks until worker threads re-enter `sleep()`).
/// This is a safety net against bugs that would otherwise cause the
/// harness to hang indefinitely.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Harness-assigned process identifier. Sequential, deterministic.
///
/// The i-th process spawned by the harness gets `HarnessId(i)`. OS PIDs
/// are nondeterministic and never appear in the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HarnessId(pub u32);

/// A process exit observed during tick settlement.
#[derive(Debug, PartialEq, Eq)]
pub struct TraceEvent {
    /// Mock time (ms since epoch) at which the exit was observed.
    pub t: u64,
    /// Which process exited (harness-assigned, not OS PID).
    pub process: HarnessId,
    /// Raw stdout from the process.
    pub stdout: Vec<u8>,
    /// Raw stderr from the process.
    pub stderr: Vec<u8>,
    /// Process exit code.
    pub exit_code: i32,
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A background child process tracked by the harness.
struct TrackedChild {
    harness_id: HarnessId,
    child: Child,
}

/// Shared state between the accept thread and the foreground test thread.
struct SharedState {
    /// Connected processes, keyed by PID. Writable by the accept thread
    /// (insert) and the foreground thread (broadcast writes, dead removal).
    connections: HashMap<u32, Connection>,
    /// PID of the daemon, if connected. Set by the accept thread when a
    /// connection with daemon argv arrives.
    daemon_pid: Option<u32>,
}

/// A connected process on the inject socket.
struct Connection {
    writer: BufWriter<UnixStream>,
    /// For reading ACK messages after `AdvanceTime`. Set with a read
    /// timeout (`ACK_TIMEOUT`) so bugs don't cause infinite hangs.
    reader: BufReader<UnixStream>,
    argv: Vec<String>,
}

// ---------------------------------------------------------------------------
// TestHarness
// ---------------------------------------------------------------------------

/// E2E test fixture: spawns processes in test mode with an isolated cache
/// dir, advances mock time, and observes process exits as trace events.
pub struct TestHarness {
    /// Path to the `attend` binary under test.
    binary: Utf8PathBuf,
    /// Isolated temp directory for all cache state.
    _cache_dir: tempfile::TempDir,
    /// Path to the cache directory (convenience reference).
    cache_path: Utf8PathBuf,
    /// Shared state with the background accept thread.
    shared: Arc<(Mutex<SharedState>, Condvar)>,
    /// Next harness-assigned process ID (sequential, deterministic).
    next_id: u32,
    /// Current mock time (ms since epoch). Starts at 0.
    mock_time_ms: u64,
    /// Background children not yet exited, keyed by OS PID.
    children: HashMap<u32, TrackedChild>,
}

impl TestHarness {
    /// Create a new harness with an isolated cache directory and inject socket.
    ///
    /// `binary` is the path to the `attend` executable under test.
    /// Spawns a background thread that runs a blocking accept loop on the
    /// inject socket.
    pub fn new(binary: impl Into<Utf8PathBuf>) -> Self {
        let binary = binary.into();
        let cache_dir = tempfile::tempdir().expect("failed to create temp cache dir");
        let cache_path =
            Utf8PathBuf::try_from(cache_dir.path().to_path_buf()).expect("non-UTF-8 temp dir");

        // Bind the inject socket before any processes are spawned.
        let sock_path = cache_path.join("test-inject.sock");
        let listener = UnixListener::bind(sock_path.as_std_path())
            .unwrap_or_else(|e| panic!("failed to bind inject socket at {sock_path}: {e}"));

        let shared = Arc::new((
            Mutex::new(SharedState {
                connections: HashMap::new(),
                daemon_pid: None,
            }),
            Condvar::new(),
        ));

        // Spawn background accept thread.
        let shared_clone = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("harness-accept".into())
            .spawn(move || accept_loop(listener, shared_clone))
            .expect("failed to spawn accept thread");

        Self {
            binary,
            _cache_dir: cache_dir,
            cache_path,
            shared,
            next_id: 0,
            mock_time_ms: 0,
            children: HashMap::new(),
        }
    }

    /// The isolated cache directory path.
    pub fn cache_dir(&self) -> &Utf8Path {
        &self.cache_path
    }

    /// The path to the `attend` binary under test.
    pub fn binary(&self) -> &Utf8Path {
        &self.binary
    }

    // -----------------------------------------------------------------------
    // Spawn (all processes are background)
    // -----------------------------------------------------------------------

    /// Spawn an attend subcommand as a background child.
    ///
    /// Waits for the child's PID to connect to the inject socket before
    /// returning. If this is a daemon-spawning command (`narrate toggle`
    /// or `narrate start`) and no daemon is currently connected, also
    /// waits for the daemon to connect.
    ///
    /// The child is tracked until it exits. Its exit will be observed by
    /// [`advance_time`](Self::advance_time) or
    /// [`collect_exits`](Self::collect_exits) as a [`TraceEvent`].
    pub fn spawn(&mut self, args: &[&str]) -> HarnessId {
        let mut child = self.spawn_command(args);
        drop(child.stdin.take()); // Close stdin (no input needed).

        let pid = child.id();
        let id = HarnessId(self.next_id);
        self.next_id += 1;

        self.wait_for_pid(pid);

        // Daemon-spawning commands fork the daemon as a detached grandchild.
        // Wait for it to connect before returning so subsequent time
        // advances reach both the daemon and the CLI command.
        let spawns_daemon = !self.has_daemon()
            && args.len() >= 2
            && args[0] == "narrate"
            && (args[1] == "toggle" || args[1] == "start");
        if spawns_daemon {
            self.wait_for_daemon();
        }

        self.children.insert(
            pid,
            TrackedChild {
                harness_id: id,
                child,
            },
        );
        id
    }

    /// Spawn an attend subcommand with stdin data as a background child.
    ///
    /// Writes `stdin_data` to the child's stdin pipe and closes it before
    /// waiting for the child to connect. The process reads stdin after
    /// connecting to the inject socket (during hook processing), so the
    /// data is already in the pipe buffer by that point.
    pub fn spawn_with_stdin(&mut self, args: &[&str], stdin_data: &[u8]) -> HarnessId {
        let mut child = self.spawn_command(args);

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(stdin_data).expect("failed to write stdin");
            // Drop closes the pipe, signaling EOF.
        }

        let pid = child.id();
        let id = HarnessId(self.next_id);
        self.next_id += 1;

        self.wait_for_pid(pid);

        self.children.insert(
            pid,
            TrackedChild {
                harness_id: id,
                child,
            },
        );
        id
    }

    // -----------------------------------------------------------------------
    // Time and injection
    // -----------------------------------------------------------------------

    /// Advance the mock clock and collect process exits.
    ///
    /// Sends `AdvanceTime` to all connected processes and waits for ACK
    /// (or connection drop) from each. Then checks all background children
    /// for exits via `try_wait()`. Returns trace events for any children
    /// that exited during this tick.
    pub fn advance_time(&mut self, duration_ms: u64) -> Vec<TraceEvent> {
        self.mock_time_ms += duration_ms;

        // Phase 0: pre-remove connections for children that exited between
        // ticks. Without this, the write may succeed (kernel-buffered) but
        // the subsequent read_line blocks for ACK_TIMEOUT (10s) waiting
        // for an ACK that will never arrive.
        let mut pre_dead = Vec::new();
        for (&pid, tracked) in &mut self.children {
            match tracked.child.try_wait() {
                Ok(Some(_)) => pre_dead.push(pid),
                _ => {}
            }
        }
        if !pre_dead.is_empty() {
            let (lock, _) = &*self.shared;
            let mut state = lock.lock().unwrap();
            for &pid in &pre_dead {
                state.connections.remove(&pid);
                if state.daemon_pid == Some(pid) {
                    state.daemon_pid = None;
                }
            }
        }

        let msg = Inject::Time(TimeInject::AdvanceTime { duration_ms });
        let json = serde_json::to_string(&msg).expect("failed to serialize AdvanceTime");
        let line = format!("{json}\n");

        let (lock, _) = &*self.shared;
        let mut state = lock.lock().unwrap();

        // Phase 1: send AdvanceTime to all connections.
        let mut dead_pids = Vec::new();
        for (&pid, conn) in &mut state.connections {
            if conn.writer.write_all(line.as_bytes()).is_err() || conn.writer.flush().is_err() {
                dead_pids.push(pid);
            }
        }

        // Phase 2: read ACK from each live connection.
        let mut ack_line = String::new();
        for (&pid, conn) in &mut state.connections {
            if dead_pids.contains(&pid) {
                continue; // Already known dead from write failure.
            }
            ack_line.clear();
            match conn.reader.read_line(&mut ack_line) {
                Ok(0) => {
                    // EOF: process exited during tick (implicit ACK).
                    dead_pids.push(pid);
                }
                Ok(_) => {
                    // Got ACK line: process has settled.
                }
                Err(_) => {
                    // Read error (timeout or broken pipe): implicit ACK.
                    dead_pids.push(pid);
                }
            }
        }

        // Remove dead connections.
        for pid in dead_pids {
            state.connections.remove(&pid);
            if state.daemon_pid == Some(pid) {
                state.daemon_pid = None;
            }
        }

        drop(state); // Release lock before collect_exits.

        // Phase 3: collect exits from background children.
        self.collect_exits(self.mock_time_ms)
    }

    /// Inject speech into the daemon's stub transcriber.
    pub fn inject_speech(&mut self, text: &str, duration_ms: u64) {
        self.broadcast_capture(&CaptureInject::Speech {
            text: text.to_string(),
            duration_ms,
        });
    }

    /// Inject a period of silence.
    pub fn inject_silence(&mut self, duration_ms: u64) {
        self.broadcast_capture(&CaptureInject::Silence { duration_ms });
    }

    /// Inject editor state.
    pub fn inject_editor_state(&mut self, files: Vec<FileEntry>) {
        self.broadcast_capture(&CaptureInject::EditorState { files });
    }

    /// Inject an external selection.
    pub fn inject_external_selection(&mut self, app: &str, text: &str) {
        self.broadcast_capture(&CaptureInject::ExternalSelection {
            app: app.to_string(),
            text: text.to_string(),
        });
    }

    /// Inject clipboard content.
    pub fn inject_clipboard(&mut self, text: &str) {
        self.broadcast_capture(&CaptureInject::Clipboard {
            text: text.to_string(),
        });
    }

    // -----------------------------------------------------------------------
    // Observation
    // -----------------------------------------------------------------------

    /// Check all background children for exits.
    ///
    /// Returns trace events for any children that exited since the last
    /// check. The timestamp `t` is attached to all returned events (it
    /// should be the current mock time).
    pub fn collect_exits(&mut self, t: u64) -> Vec<TraceEvent> {
        let mut exited = Vec::new();
        for (&pid, tracked) in &mut self.children {
            match tracked.child.try_wait() {
                Ok(Some(status)) => exited.push((pid, status)),
                Ok(None) => {}
                Err(e) => panic!("try_wait failed for PID {pid}: {e}"),
            }
        }

        let mut events = Vec::new();
        for (pid, status) in exited {
            let tracked = self.children.remove(&pid).unwrap();
            let output = collect_output(tracked.child, status);
            self.remove_connection(pid);
            events.push(TraceEvent {
                t,
                process: tracked.harness_id,
                stdout: output.stdout,
                stderr: output.stderr,
                exit_code: status.code().unwrap_or(-1),
            });
        }

        events
    }

    /// Advance time in [`TICK_MS`] increments until a specific process exits.
    ///
    /// Returns the exit event for the target process. Events from other
    /// processes that exit during the ticks are discarded (their children
    /// are still properly cleaned up).
    ///
    /// Panics if the process doesn't exit within [`COMMAND_TIMEOUT`]
    /// wall-clock seconds.
    pub fn tick_until_exit(&mut self, id: HarnessId) -> TraceEvent {
        let start = std::time::Instant::now();
        loop {
            let events = self.advance_time(TICK_MS);
            for event in events {
                if event.process == id {
                    return event;
                }
            }

            // Verify the target is still tracked (hasn't already exited
            // in an earlier advance_time call that the caller ignored).
            if !self.children.values().any(|tc| tc.harness_id == id) {
                panic!("{id:?} is not a tracked child (already exited or never spawned)");
            }

            if start.elapsed() > COMMAND_TIMEOUT {
                panic!(
                    "timed out waiting for {id:?} to exit after {:.1?} wall-clock",
                    start.elapsed()
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Shared state access
    // -----------------------------------------------------------------------

    /// Check whether a daemon is connected.
    pub fn has_daemon(&self) -> bool {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().daemon_pid.is_some()
    }

    /// Spawn an attend subcommand as a child process (internal).
    fn spawn_command(&self, args: &[&str]) -> Child {
        let mut cmd = Command::new(self.binary.as_str());
        cmd.args(args);
        cmd.env("ATTEND_TEST_MODE", "1");
        cmd.env("ATTEND_CACHE_DIR", self.cache_path.as_str());
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {:?}: {e}", args))
    }

    /// Block until a specific PID connects to the inject socket.
    fn wait_for_pid(&self, target_pid: u32) {
        let (lock, cvar) = &*self.shared;
        let guard = lock.lock().unwrap();
        let (guard, timeout) = cvar
            .wait_timeout_while(guard, CONNECT_TIMEOUT, |state| {
                !state.connections.contains_key(&target_pid)
            })
            .unwrap();
        if timeout.timed_out() {
            panic!(
                "timed out waiting for PID {target_pid} to connect (connected: {:?})",
                guard.connections.keys().collect::<Vec<_>>()
            );
        }
    }

    /// Block until a daemon process connects to the inject socket.
    fn wait_for_daemon(&self) {
        let (lock, cvar) = &*self.shared;
        let guard = lock.lock().unwrap();
        let (_guard, timeout) = cvar
            .wait_timeout_while(guard, CONNECT_TIMEOUT, |state| state.daemon_pid.is_none())
            .unwrap();
        if timeout.timed_out() {
            panic!("timed out waiting for daemon to connect");
        }
    }

    /// Remove a connection (process has exited).
    fn remove_connection(&self, pid: u32) {
        let (lock, _) = &*self.shared;
        let mut state = lock.lock().unwrap();
        state.connections.remove(&pid);
        if state.daemon_pid == Some(pid) {
            state.daemon_pid = None;
        }
    }

    /// Broadcast a capture injection to all connected processes
    /// (fire-and-forget, no ACK). Time advances must go through
    /// `advance_time()` which handles the ACK protocol.
    fn broadcast_capture(&self, msg: &CaptureInject) {
        let json = serde_json::to_string(msg).expect("failed to serialize capture injection");
        let line = format!("{json}\n");

        let (lock, _) = &*self.shared;
        let mut state = lock.lock().unwrap();

        let mut dead_pids = Vec::new();
        for (&pid, conn) in &mut state.connections {
            if conn.writer.write_all(line.as_bytes()).is_err() || conn.writer.flush().is_err() {
                dead_pids.push(pid);
            }
        }

        for pid in dead_pids {
            state.connections.remove(&pid);
            if state.daemon_pid == Some(pid) {
                state.daemon_pid = None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Background accept loop
// ---------------------------------------------------------------------------

/// Blocking accept loop: runs on a background thread, accepts connections,
/// reads handshakes, inserts into shared state, notifies condvar.
fn accept_loop(listener: UnixListener, shared: Arc<(Mutex<SharedState>, Condvar)>) {
    loop {
        let (stream, _) = match listener.accept() {
            Ok(conn) => conn,
            Err(_) => break, // Listener dropped or error — exit thread.
        };

        // Read the handshake (one newline-delimited JSON line).
        // The process sends this immediately on connect, so this blocks
        // only briefly.
        let reader_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut reader = BufReader::new(reader_stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            continue;
        }

        let handshake: Handshake = match serde_json::from_str(&line) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("harness: invalid handshake JSON: {e}\nraw: {line}");
                continue;
            }
        };

        let pid = handshake.pid;
        let is_daemon = handshake
            .argv
            .iter()
            .any(|a| a.contains("_record-daemon") || a.contains("_daemon"));

        // Set read timeout on the reader stream for future ACK reads.
        // The handshake has already been consumed; subsequent reads
        // will be ACK lines sent by the process after AdvanceTime.
        reader.get_ref().set_read_timeout(Some(ACK_TIMEOUT)).ok();

        let (lock, cvar) = &*shared;
        let mut state = lock.lock().unwrap();
        if is_daemon {
            state.daemon_pid = Some(pid);
        }
        state.connections.insert(
            pid,
            Connection {
                writer: BufWriter::new(stream),
                reader,
                argv: handshake.argv,
            },
        );
        cvar.notify_all();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Collect stdout and stderr from a child that has already exited.
fn collect_output(mut child: Child, status: ExitStatus) -> Output {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(ref mut out) = child.stdout {
        let _ = out.read_to_end(&mut stdout);
    }
    if let Some(ref mut err) = child.stderr {
        let _ = err.read_to_end(&mut stderr);
    }
    Output {
        status,
        stdout,
        stderr,
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        // Kill all tracked children (ephemeral CLI commands still running).
        for (_, tracked) in &mut self.children {
            let _ = tracked.child.kill();
        }

        // Send SIGTERM to the daemon if it's still running. The daemon is
        // a grandchild (not in `self.children`) — it's tracked via the
        // inject socket connection, so we signal it by PID.
        let (lock, _) = &*self.shared;
        if let Some(pid) = lock.lock().unwrap().daemon_pid {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        // The background accept thread exits when the listener is dropped
        // (which happens when _cache_dir drops and the socket file disappears,
        // or when the thread sees an accept error).
    }
}
