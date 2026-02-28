//! Test mode infrastructure for deterministic end-to-end testing.
//!
//! When `ATTEND_TEST_MODE=1`, the binary swaps production capture sources
//! for stubs driven by an inject socket. The test harness controls time,
//! audio transcription, editor state, clipboard content, and external
//! selections — all via messages over `$ATTEND_CACHE_DIR/test-inject.sock`.
//!
//! # Architecture
//!
//! The **harness** is the server: it binds the inject socket and broadcasts
//! messages to all connected processes. Every process spawned with
//! `ATTEND_TEST_MODE=1` connects to the inject socket at startup, sends a
//! handshake (PID + argv), and reads `Inject` messages on a background thread.
//!
//! `AdvanceTime` messages go to the process-wide `MockClock`. Capture
//! injections (speech, editor, clipboard, ext) go to stub channels that
//! only the daemon routes to its capture sources. Non-daemon processes
//! silently ignore capture injections.
//!
//! # Invariant
//!
//! The inject socket background thread must **never** call `clock.sleep()`.
//! It is the only thread that calls `MockClock::advance()`. If it blocked
//! on the condvar, no thread could wake it — deadlock.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::clock::MockClock;
use crate::narrate::ext_capture::ExternalSnapshot;
use crate::narrate::transcribe::stub::Injection;
use crate::state::{EditorState, FileEntry};

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Process-wide MockClock, set once by `init()`.
static CLOCK: OnceLock<Arc<MockClock>> = OnceLock::new();

/// Inject router, set once by `init()`. Routes capture injections from
/// the inject socket to shared state that stubs read from. Registered
/// at the top of main so no messages are lost during daemon initialization.
static INJECT_ROUTER: OnceLock<InjectRouter> = OnceLock::new();

/// StubTranscriber created during `init()`, taken once by the daemon via
/// `take_stub_transcriber()`. Non-daemon processes leave this untouched.
static STUB_TRANSCRIBER: OnceLock<
    Mutex<Option<crate::narrate::transcribe::stub::StubTranscriber>>,
> = OnceLock::new();

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check if test mode is active.
pub fn is_active() -> bool {
    std::env::var("ATTEND_TEST_MODE").is_ok_and(|v| v == "1")
}

/// Return the test-mode MockClock, if initialized.
pub fn clock() -> Option<Arc<MockClock>> {
    CLOCK.get().cloned()
}

/// Initialize test mode: create MockClock, inject router, and stub
/// transcriber. Does NOT connect to the inject socket — call `connect()`
/// for that.
///
/// Must be called at the top of `main()`, before any clock usage.
pub fn init() {
    // Create and store the mock clock (start at Unix epoch).
    let start = chrono::DateTime::UNIX_EPOCH;
    let clock = Arc::new(MockClock::new(start));
    CLOCK
        .set(Arc::clone(&clock))
        .expect("test_mode::init called twice");

    // Create the inject router and stub transcriber. The router is
    // registered now so the reader thread (started by connect()) can
    // dispatch from its first message. The StubTranscriber is stored
    // for the daemon to take via take_stub_transcriber().
    let editor_state: Arc<Mutex<Option<crate::state::EditorState>>> = Arc::default();
    let ext_snapshot: Arc<Mutex<Option<crate::narrate::ext_capture::ExternalSnapshot>>> =
        Arc::default();
    let clipboard_text: Arc<Mutex<Option<String>>> = Arc::default();
    let (stub_transcriber, transcriber_tx) =
        crate::narrate::transcribe::stub::StubTranscriber::new();

    INJECT_ROUTER
        .set(InjectRouter {
            transcriber_tx,
            editor_state,
            ext_snapshot,
            clipboard_text,
        })
        .expect("inject router already registered");
    STUB_TRANSCRIBER
        .set(Mutex::new(Some(stub_transcriber)))
        .expect("stub transcriber already set");
}

/// Connect to the harness's inject socket, send the handshake, and spawn
/// the background reader thread.
///
/// Non-daemon processes call this at the top of main (right after `init()`).
/// The daemon calls this after initialization is complete, so the harness
/// knows "daemon connected" means "daemon ready."
pub fn connect() {
    let cache_dir = crate::state::cache_dir().expect("ATTEND_CACHE_DIR must be set in test mode");
    let sock_path = cache_dir.join("test-inject.sock");

    let clock = CLOCK
        .get()
        .expect("test_mode::connect called before init")
        .clone();

    let mut stream = UnixStream::connect(sock_path.as_std_path())
        .unwrap_or_else(|e| panic!("failed to connect to inject socket at {sock_path}: {e}"));

    let handshake = Handshake {
        pid: std::process::id(),
        argv: std::env::args().collect(),
    };
    serde_json::to_writer(&stream, &handshake).expect("failed to write handshake");
    stream.write_all(b"\n").expect("failed to write newline");
    stream.flush().expect("failed to flush handshake");

    // Clone the stream: original for reading (BufReader), clone for
    // writing ACKs back to the harness after AdvanceTime settlement.
    let ack_stream = stream
        .try_clone()
        .expect("failed to clone inject socket for ACK writes");

    std::thread::Builder::new()
        .name("test-inject".into())
        .spawn(move || reader_loop(stream, ack_stream, clock))
        .expect("failed to spawn inject reader thread");
}

/// Take the stub transcriber created during `init()`. Called once by the
/// daemon during capture setup. Panics if not in test mode or already taken.
pub fn take_stub_transcriber() -> crate::narrate::transcribe::stub::StubTranscriber {
    STUB_TRANSCRIBER
        .get()
        .expect("test_mode::init not called")
        .lock()
        .unwrap()
        .take()
        .expect("stub transcriber already taken")
}

/// Get the inject router (for `CaptureConfig::test_mode()` to read shared state).
pub fn router() -> &'static InjectRouter {
    INJECT_ROUTER.get().expect("test_mode::init not called")
}

// ---------------------------------------------------------------------------
// Background reader thread
// ---------------------------------------------------------------------------

/// Read inject messages from the harness and dispatch them.
///
/// Runs until the connection closes (process exit or harness teardown).
/// Never calls `clock.sleep()` — see module-level invariant.
///
/// On `AdvanceTime`: calls `advance_and_settle()` which bumps the clock
/// and blocks on the settlement condvar until all woken threads have
/// re-entered `sleep()` (or departed), then writes `{"ack":true}\n`
/// back to the harness. This is the process-side half of the ACK
/// protocol — the harness waits for ACKs from every connected process
/// before proceeding.
fn reader_loop(stream: UnixStream, ack_stream: UnixStream, clock: Arc<MockClock>) {
    let reader = BufReader::new(stream);
    let mut ack_writer = BufWriter::new(ack_stream);
    // The router is registered during init(), before this thread starts
    // reading, so it's always available.
    let router = INJECT_ROUTER.get();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // Connection closed.
        };
        if line.is_empty() {
            continue;
        }
        let msg: Inject = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("inject socket: bad message: {e}");
                continue;
            }
        };
        match msg {
            Inject::AdvanceTime { duration_ms } => {
                // ACK protocol: advance time and block until all woken
                // threads have re-entered sleep(). See phase-20-testing.md
                // §Tick synchronization.
                clock.advance_and_settle(Duration::from_millis(duration_ms));

                // If the process is exiting (a woken thread caused
                // main() to return), wait_for_waiters blocks until
                // this thread is killed. The socket drop serves as
                // an implicit ACK to the harness.
                let _ = ack_writer.write_all(b"{\"ack\":true}\n");
                let _ = ack_writer.flush();
            }
            capture_msg => {
                if let Some(r) = router {
                    r.dispatch(capture_msg);
                }
                // Non-daemon processes have no router; silently ignore.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Inject router (daemon only)
// ---------------------------------------------------------------------------

/// Routes capture injections from the inject socket to the daemon's
/// stub channels. Non-daemon processes never register a router.
#[derive(Debug)]
pub struct InjectRouter {
    pub transcriber_tx: std::sync::mpsc::Sender<Injection>,
    pub editor_state: Arc<Mutex<Option<EditorState>>>,
    pub ext_snapshot: Arc<Mutex<Option<ExternalSnapshot>>>,
    pub clipboard_text: Arc<Mutex<Option<String>>>,
}

impl InjectRouter {
    fn dispatch(&self, msg: Inject) {
        match msg {
            Inject::Speech { text, duration_ms } => {
                let _ = self.transcriber_tx.send(Injection { text, duration_ms });
            }
            Inject::Silence { duration_ms } => {
                let _ = self.transcriber_tx.send(Injection {
                    text: String::new(),
                    duration_ms,
                });
            }
            Inject::EditorState { files } => {
                *self.editor_state.lock().unwrap() = Some(EditorState { files, cwd: None });
            }
            Inject::ExternalSelection { app, text } => {
                *self.ext_snapshot.lock().unwrap() = Some(ExternalSnapshot {
                    app,
                    window_title: String::new(),
                    selected_text: Some(text),
                });
            }
            Inject::Clipboard { text } => {
                *self.clipboard_text.lock().unwrap() = Some(text);
            }
            Inject::AdvanceTime { .. } => unreachable!("handled before dispatch"),
        }
    }
}

// ---------------------------------------------------------------------------
// Inject socket protocol
// ---------------------------------------------------------------------------

/// Handshake sent by each process on connecting to the inject socket.
///
/// The harness uses `pid` for spawn-connect synchronization (blocking
/// until a spawned PID connects) and `argv` to identify the daemon
/// (whose argv ends with `narrate _daemon`) vs CLI commands.
#[derive(Debug, Serialize, Deserialize)]
pub struct Handshake {
    pub pid: u32,
    pub argv: Vec<String>,
}

/// Harness → Process injection message (newline-delimited JSON).
///
/// All messages are broadcast to every connected process. The daemon
/// routes capture injections to its stub channels; non-daemon processes
/// ignore them. `AdvanceTime` is meaningful to all processes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Inject {
    /// Advance the mock clock by this duration. Wakes any threads
    /// blocked in `MockClock::sleep()` whose deadline is now met.
    AdvanceTime { duration_ms: u64 },

    /// Inject speech: what was said and how long it took.
    /// Daemon routes to `StubTranscriber`; others ignore.
    Speech { text: String, duration_ms: u64 },

    /// Inject a period of silence.
    /// Daemon routes to `StubTranscriber`; others ignore.
    Silence { duration_ms: u64 },

    /// Stub editor capture returns this state on next poll.
    /// Daemon routes to `StubEditorSource`; others ignore.
    EditorState { files: Vec<FileEntry> },

    /// Stub ext capture returns this selection on next poll.
    /// Daemon routes to `StubExternalSource`; others ignore.
    ExternalSelection { app: String, text: String },

    /// Stub clipboard capture emits this content on next poll.
    /// Daemon routes to `StubClipboardSource`; others ignore.
    Clipboard { text: String },
}

pub mod stubs;

#[cfg(test)]
mod tests;
