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
//! # Time coordination
//!
//! All processes under test use a `MockClock` with condvar-gated sleep.
//! The harness advances time by broadcasting `AdvanceTime` messages.
//! When waiting for a CLI command to exit, the harness ticks time forward
//! in small increments so both the daemon and the CLI make progress.

mod protocol;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};

pub use protocol::{FileEntry, Inject};
use protocol::Handshake;

// ---------------------------------------------------------------------------
// Configuration constants
// ---------------------------------------------------------------------------

/// How long (wall-clock) to wait for a process to connect to the inject socket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long (wall-clock) to wait for a CLI command to exit.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Mock time increment per tick while waiting for a child to exit (ms).
/// Must be <= the smallest poll interval in the codebase (50ms for
/// sentinel polling).
const TICK_MS: u64 = 50;

/// Small real-time delay between ticks to avoid busy-spinning.
const TICK_REAL_DELAY: Duration = Duration::from_millis(5);

// ---------------------------------------------------------------------------
// TestHarness
// ---------------------------------------------------------------------------

/// E2E test fixture: spawns a daemon in test mode with an isolated cache
/// dir, provides helpers to invoke CLI commands and assert on outputs.
pub struct TestHarness {
    /// Path to the `attend` binary under test.
    binary: Utf8PathBuf,
    /// Isolated temp directory for all cache state.
    _cache_dir: tempfile::TempDir,
    /// Path to the cache directory (convenience reference).
    cache_path: Utf8PathBuf,
    /// Inject socket listener, bound before any process is spawned.
    inject_listener: UnixListener,
    /// Connected processes, keyed by PID.
    connections: HashMap<u32, Connection>,
    /// PID of the daemon process, if connected.
    daemon_pid: Option<u32>,
}

/// A connected process on the inject socket.
struct Connection {
    writer: BufWriter<UnixStream>,
    #[allow(dead_code)]
    argv: Vec<String>,
}

impl TestHarness {
    /// Create a new harness with an isolated cache directory and inject socket.
    ///
    /// `binary` is the path to the `attend` executable under test.
    pub fn new(binary: impl Into<Utf8PathBuf>) -> Self {
        let binary = binary.into();
        let cache_dir = tempfile::tempdir().expect("failed to create temp cache dir");
        let cache_path =
            Utf8PathBuf::try_from(cache_dir.path().to_path_buf()).expect("non-UTF-8 temp dir");

        // Bind the inject socket before any processes are spawned.
        let sock_path = cache_path.join("test-inject.sock");
        let inject_listener = UnixListener::bind(sock_path.as_std_path())
            .unwrap_or_else(|e| panic!("failed to bind inject socket at {sock_path}: {e}"));

        Self {
            binary,
            _cache_dir: cache_dir,
            cache_path,
            inject_listener,
            connections: HashMap::new(),
            daemon_pid: None,
        }
    }

    /// The isolated cache directory path.
    pub fn cache_dir(&self) -> &Utf8Path {
        &self.cache_path
    }

    // -----------------------------------------------------------------------
    // CLI command helpers
    // -----------------------------------------------------------------------

    /// Spawn an attend subcommand, wait for it to connect to the inject
    /// socket, then tick time until it exits.
    fn run_command(&mut self, args: &[&str]) -> Output {
        let child = self.spawn_command(args);
        let pid = child.id();
        self.wait_for_pid(pid);
        self.wait_child_ticking(child)
    }

    /// Spawn an attend subcommand as a child process.
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

    /// Spawn a command with stdin data, wait for connect, tick until exit.
    fn run_command_with_stdin(&mut self, args: &[&str], stdin_data: &str) -> Output {
        let mut child = self.spawn_command(args);

        // Write stdin and close it before waiting for connect.
        // The process reads stdin after connecting to the inject socket
        // (during hook processing), so the data must be in the pipe buffer.
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(stdin_data.as_bytes())
                .expect("failed to write stdin");
            // Drop closes the pipe, signaling EOF.
        }

        let pid = child.id();
        self.wait_for_pid(pid);
        self.wait_child_ticking(child)
    }

    /// Wait for a child process to exit, advancing mock time in small
    /// increments so all processes (daemon + CLI) can make progress.
    ///
    /// Also accepts any new connections that arrive during the wait
    /// (e.g., the daemon connecting as a grandchild of toggle).
    fn wait_child_ticking(&mut self, mut child: Child) -> Output {
        let pid = child.id();
        let deadline = std::time::Instant::now() + COMMAND_TIMEOUT;

        loop {
            // Check for new connections (non-blocking).
            self.drain_pending_connections();

            match child.try_wait() {
                Ok(Some(status)) => {
                    // Child exited. Collect remaining output.
                    let output = collect_output(child, status);
                    self.connections.remove(&pid);
                    return output;
                }
                Ok(None) => {
                    // Child still running. Advance mock time.
                    self.broadcast(&Inject::AdvanceTime {
                        duration_ms: TICK_MS,
                    });
                    std::thread::sleep(TICK_REAL_DELAY);
                }
                Err(e) => panic!("try_wait failed for PID {pid}: {e}"),
            }

            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let stderr = child
                    .stderr
                    .as_mut()
                    .map(|s| {
                        let mut buf = Vec::new();
                        let _ = s.read_to_end(&mut buf);
                        String::from_utf8_lossy(&buf).to_string()
                    })
                    .unwrap_or_default();
                panic!("timed out waiting for child PID {pid} to exit\nstderr: {stderr}");
            }
        }
    }

    /// Toggle recording (start if idle, stop if recording).
    pub fn toggle(&mut self) -> Output {
        let had_daemon = self.daemon_pid.is_some();
        let output = self.run_command(&["narrate", "toggle"]);
        // If this is the first toggle, the daemon was spawned as a
        // grandchild. Wait for it to connect.
        if !had_daemon && self.daemon_pid.is_none() {
            self.wait_for_daemon();
        }
        output
    }

    /// Start recording (no-op if already recording).
    pub fn start(&mut self) -> Output {
        let had_daemon = self.daemon_pid.is_some();
        let output = self.run_command(&["narrate", "start"]);
        if !had_daemon && self.daemon_pid.is_none() {
            self.wait_for_daemon();
        }
        output
    }

    /// Stop recording.
    pub fn stop(&mut self) -> Output {
        self.run_command(&["narrate", "stop"])
    }

    /// Pause/resume recording.
    pub fn pause(&mut self) -> Output {
        self.run_command(&["narrate", "pause"])
    }

    /// Yank (finalize + copy to clipboard).
    pub fn yank(&mut self) -> Output {
        self.run_command(&["narrate", "yank"])
    }

    /// Query daemon status. Returns the raw stdout.
    pub fn status(&mut self) -> String {
        let output = self.run_command(&["narrate", "status"]);
        String::from_utf8(output.stdout).expect("non-UTF-8 status output")
    }

    /// Send a browser selection event via the native messaging bridge.
    ///
    /// Invokes `attend browser-bridge` with native-messaging-formatted
    /// stdin (4-byte LE length prefix + JSON).
    pub fn browser_event(&mut self, url: &str, title: &str, html: &str, plain_text: &str) {
        let msg = serde_json::json!({
            "url": url,
            "title": title,
            "html": html,
            "plain_text": plain_text,
        });
        let json_bytes = serde_json::to_vec(&msg).expect("failed to serialize browser event");
        let len = json_bytes.len() as u32;

        let mut stdin_data = Vec::with_capacity(4 + json_bytes.len());
        stdin_data.extend_from_slice(&len.to_le_bytes());
        stdin_data.extend_from_slice(&json_bytes);

        let mut child = self.spawn_command(&["browser-bridge"]);
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&stdin_data)
                .expect("failed to write native messaging stdin");
        }
        let pid = child.id();
        self.wait_for_pid(pid);
        let output = self.wait_child_ticking(child);
        assert!(
            output.status.success(),
            "browser-bridge failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Send a shell command event (postexec: command completed).
    ///
    /// Invokes `attend shell-hook postexec` with the given parameters.
    pub fn shell_event(
        &mut self,
        shell: &str,
        command: &str,
        exit_status: i32,
        duration_secs: f64,
    ) {
        let output = self.run_command(&[
            "shell-hook",
            "postexec",
            "--shell",
            shell,
            "--command",
            command,
            "--exit-status",
            &exit_status.to_string(),
            "--duration",
            &duration_secs.to_string(),
        ]);
        assert!(
            output.status.success(),
            "shell-hook failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // -----------------------------------------------------------------------
    // Hook helpers
    // -----------------------------------------------------------------------

    /// Activate a narration session (simulates the `/attend` user prompt hook).
    ///
    /// This invokes `attend hook user-prompt -a claude` with stdin JSON
    /// containing `{"prompt": "/attend", "session_id": "...", "cwd": "..."}`.
    pub fn activate_session(&mut self, session_id: &str) -> Output {
        let stdin = serde_json::json!({
            "session_id": session_id,
            "cwd": self.cache_path.as_str(),
            "prompt": "/attend",
        });
        self.run_command_with_stdin(
            &["hook", "user-prompt", "-a", "claude"],
            &serde_json::to_string(&stdin).unwrap(),
        )
    }

    /// Deactivate a narration session (simulates `/unattend` user prompt).
    pub fn deactivate_session(&mut self, session_id: &str) -> Output {
        let stdin = serde_json::json!({
            "session_id": session_id,
            "cwd": self.cache_path.as_str(),
            "prompt": "/unattend",
        });
        self.run_command_with_stdin(
            &["hook", "user-prompt", "-a", "claude"],
            &serde_json::to_string(&stdin).unwrap(),
        )
    }

    /// Collect pending narration via the PreToolUse hook.
    ///
    /// This invokes `attend hook pre-tool-use -a claude` with stdin JSON
    /// simulating an `attend listen` Bash command. Returns the raw stdout
    /// which contains the rendered narration (if any).
    pub fn collect(&mut self, session_id: &str) -> String {
        let stdin = serde_json::json!({
            "session_id": session_id,
            "cwd": self.cache_path.as_str(),
            "tool_name": "Bash",
            "tool_input": {
                "command": format!("{} listen --wait --session {}", self.binary, session_id),
            },
        });
        let output = self.run_command_with_stdin(
            &["hook", "pre-tool-use", "-a", "claude"],
            &serde_json::to_string(&stdin).unwrap(),
        );
        String::from_utf8(output.stdout).expect("non-UTF-8 hook output")
    }

    /// Fire a PreToolUse hook for a non-listen tool (e.g., to check for
    /// narration delivery on any tool use).
    pub fn fire_pre_tool_use(&mut self, session_id: &str) -> String {
        let stdin = serde_json::json!({
            "session_id": session_id,
            "cwd": self.cache_path.as_str(),
            "tool_name": "Read",
            "tool_input": {
                "file_path": "/tmp/example.rs",
            },
        });
        let output = self.run_command_with_stdin(
            &["hook", "pre-tool-use", "-a", "claude"],
            &serde_json::to_string(&stdin).unwrap(),
        );
        String::from_utf8(output.stdout).expect("non-UTF-8 hook output")
    }

    /// Start `attend listen` as a background process.
    ///
    /// Returns the child handle. The listener blocks until narration is
    /// ready, then exits.
    pub fn listen(&mut self, session_id: &str) -> Child {
        let child = self.spawn_command(&["listen", "--wait", "--session", session_id]);
        let pid = child.id();
        self.wait_for_pid(pid);
        child
    }

    /// Wait for a previously spawned child to exit, ticking time.
    pub fn wait_child(&mut self, child: Child) -> Output {
        self.wait_child_ticking(child)
    }

    // -----------------------------------------------------------------------
    // Injection helpers (broadcast to all connected processes)
    // -----------------------------------------------------------------------

    /// Advance the mock clock for all connected processes.
    pub fn advance_time(&mut self, duration_ms: u64) {
        self.broadcast(&Inject::AdvanceTime { duration_ms });
    }

    /// Inject speech into the daemon's stub transcriber.
    pub fn inject_speech(&mut self, text: &str, duration_ms: u64) {
        self.broadcast(&Inject::Speech {
            text: text.to_string(),
            duration_ms,
        });
    }

    /// Inject a period of silence.
    pub fn inject_silence(&mut self, duration_ms: u64) {
        self.broadcast(&Inject::Silence { duration_ms });
    }

    /// Inject editor state.
    pub fn inject_editor_state(&mut self, files: Vec<FileEntry>) {
        self.broadcast(&Inject::EditorState { files });
    }

    /// Inject an external selection.
    pub fn inject_external_selection(&mut self, app: &str, text: &str) {
        self.broadcast(&Inject::ExternalSelection {
            app: app.to_string(),
            text: text.to_string(),
        });
    }

    /// Inject clipboard content.
    pub fn inject_clipboard(&mut self, text: &str) {
        self.broadcast(&Inject::Clipboard {
            text: text.to_string(),
        });
    }

    // -----------------------------------------------------------------------
    // Connection management
    // -----------------------------------------------------------------------

    /// Accept a new connection, read handshake, store it. Returns the PID.
    fn accept_connection(&mut self, stream: UnixStream) -> u32 {
        // Switch to blocking mode for the handshake read. The process
        // sends the handshake immediately after connecting (top of main),
        // so this doesn't need a timeout.
        stream
            .set_nonblocking(false)
            .expect("failed to set stream to blocking");

        let reader_stream = stream
            .try_clone()
            .expect("failed to clone stream for reader");
        let mut reader = BufReader::new(reader_stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .expect("failed to read handshake");

        let handshake: Handshake = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("invalid handshake JSON: {e}\nraw: {line}"));

        let pid = handshake.pid;
        let is_daemon = handshake
            .argv
            .iter()
            .any(|a| a.contains("_record-daemon") || a.contains("_daemon"));
        if is_daemon {
            self.daemon_pid = Some(pid);
        }

        self.connections.insert(
            pid,
            Connection {
                writer: BufWriter::new(stream),
                argv: handshake.argv,
            },
        );

        pid
    }

    /// Non-blocking: accept any pending connections without waiting.
    fn drain_pending_connections(&mut self) {
        self.inject_listener
            .set_nonblocking(true)
            .expect("failed to set nonblocking");

        loop {
            match self.inject_listener.accept() {
                Ok((stream, _)) => {
                    self.accept_connection(stream);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("inject socket accept failed: {e}"),
            }
        }
    }

    /// Block until a specific PID connects to the inject socket.
    fn wait_for_pid(&mut self, target_pid: u32) {
        if self.connections.contains_key(&target_pid) {
            return;
        }

        self.inject_listener
            .set_nonblocking(true)
            .expect("failed to set nonblocking");

        let deadline = std::time::Instant::now() + CONNECT_TIMEOUT;
        while std::time::Instant::now() < deadline {
            match self.inject_listener.accept() {
                Ok((stream, _)) => {
                    let pid = self.accept_connection(stream);
                    if pid == target_pid {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("inject socket accept failed: {e}"),
            }
        }
        panic!(
            "timed out waiting for PID {target_pid} to connect (connected: {:?})",
            self.connections.keys().collect::<Vec<_>>()
        );
    }

    /// Wait for a new daemon connection (unknown PID with daemon argv).
    fn wait_for_daemon(&mut self) {
        if self.daemon_pid.is_some() {
            return;
        }

        self.inject_listener
            .set_nonblocking(true)
            .expect("failed to set nonblocking");

        let deadline = std::time::Instant::now() + CONNECT_TIMEOUT;
        while std::time::Instant::now() < deadline {
            match self.inject_listener.accept() {
                Ok((stream, _)) => {
                    self.accept_connection(stream);
                    if self.daemon_pid.is_some() {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("inject socket accept failed: {e}"),
            }
        }
        panic!("timed out waiting for daemon to connect to inject socket");
    }

    /// Broadcast an inject message to all connected processes.
    fn broadcast(&mut self, msg: &Inject) {
        let json = serde_json::to_string(msg).expect("failed to serialize inject message");
        let line = format!("{json}\n");

        let mut dead_pids = Vec::new();
        for (&pid, conn) in &mut self.connections {
            if conn.writer.write_all(line.as_bytes()).is_err()
                || conn.writer.flush().is_err()
            {
                dead_pids.push(pid);
            }
        }

        for pid in dead_pids {
            self.connections.remove(&pid);
            if self.daemon_pid == Some(pid) {
                self.daemon_pid = None;
            }
        }
    }
}

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
        // Send SIGTERM to the daemon if it's still running.
        if let Some(pid) = self.daemon_pid {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
            // Give the daemon a moment to clean up.
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
