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

use std::io::{BufRead, BufReader, Write};
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

/// Daemon's inject router, set once by `register_router()`.
static INJECT_ROUTER: OnceLock<InjectRouter> = OnceLock::new();

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

/// Initialize test mode: create MockClock, connect to inject socket,
/// spawn background reader thread.
///
/// Must be called at the top of `main()`, before any clock usage.
/// Panics if `ATTEND_CACHE_DIR` is not set or the inject socket is
/// unreachable.
pub fn init() {
    let cache_dir = crate::state::cache_dir().expect("ATTEND_CACHE_DIR must be set in test mode");
    let sock_path = cache_dir.join("test-inject.sock");

    // Create and store the mock clock (start at Unix epoch).
    let start = chrono::DateTime::UNIX_EPOCH;
    let clock = Arc::new(MockClock::new(start));
    CLOCK
        .set(Arc::clone(&clock))
        .expect("test_mode::init called twice");

    // Connect to inject socket.
    let mut stream = UnixStream::connect(sock_path.as_std_path())
        .unwrap_or_else(|e| panic!("failed to connect to inject socket at {sock_path}: {e}"));

    // Send handshake (newline-delimited JSON).
    let handshake = Handshake {
        pid: std::process::id(),
        argv: std::env::args().collect(),
    };
    serde_json::to_writer(&stream, &handshake).expect("failed to write handshake");
    stream.write_all(b"\n").expect("failed to write newline");
    stream.flush().expect("failed to flush handshake");

    // Spawn background reader thread.
    std::thread::Builder::new()
        .name("test-inject".into())
        .spawn(move || reader_loop(stream, clock))
        .expect("failed to spawn inject reader thread");
}

/// Register the daemon's inject router.
///
/// Called by the daemon after setting up capture stubs. The background
/// reader thread routes capture injections to the router's shared state.
pub fn register_router(router: InjectRouter) {
    if INJECT_ROUTER.set(router).is_err() {
        panic!("inject router already registered");
    }
}

// ---------------------------------------------------------------------------
// Background reader thread
// ---------------------------------------------------------------------------

/// Read inject messages from the harness and dispatch them.
///
/// Runs until the connection closes (process exit or harness teardown).
/// Never calls `clock.sleep()` — see module-level invariant.
fn reader_loop(stream: UnixStream, clock: Arc<MockClock>) {
    let reader = BufReader::new(stream);
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
                clock.advance(Duration::from_millis(duration_ms));
            }
            capture_msg => {
                // Route to daemon stubs if registered; silently ignore otherwise.
                if let Some(router) = INJECT_ROUTER.get() {
                    router.dispatch(capture_msg);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Inject router (daemon only)
// ---------------------------------------------------------------------------

/// Routes capture injections from the inject socket to the daemon's
/// stub channels. Non-daemon processes never register a router.
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

// ---------------------------------------------------------------------------
// Stub capture sources
// ---------------------------------------------------------------------------

pub mod stubs {
    //! Stub implementations of capture source traits for test mode.
    //!
    //! Each stub holds shared state (`Arc<Mutex<...>>`) that the inject
    //! router writes to and the capture thread reads from. This decouples
    //! the inject socket background thread from the capture polling loops.
    //!
    //! **Why mutexes (latest-wins) rather than channels?** Editor state,
    //! clipboard content, and external selections are *snapshots*, not
    //! events. In production, if the editor changes three times between
    //! polls, only the final state is observed. The mutex model faithfully
    //! mirrors this: inject sets the current state, and every poll returns
    //! it until overwritten. The harness controls time via condvar-gated
    //! `MockClock`, so it can advance by exactly one poll interval between
    //! injections when ordering matters.
    //!
    //! Speech/silence injections are different — they're events that must
    //! not be lost. Those use a channel (`mpsc::Sender`) in
    //! [`StubTranscriber`](crate::narrate::transcribe::stub::StubTranscriber),
    //! which drains all pending injections on each `transcribe()` call.

    use std::sync::{Arc, Mutex};

    use camino::{Utf8Path, Utf8PathBuf};

    use crate::narrate::audio::{AudioChunk, AudioSource, Recording};
    use crate::narrate::clipboard_capture::{ClipboardSource, ImageData};
    use crate::narrate::editor_capture::EditorStateSource;
    use crate::narrate::ext_capture::{ExternalSnapshot, ExternalSource};
    use crate::state::EditorState;

    // -- Editor ---------------------------------------------------------------

    /// Stub editor source: returns the most recently injected state.
    pub struct StubEditorSource {
        state: Arc<Mutex<Option<EditorState>>>,
    }

    impl StubEditorSource {
        pub fn new(state: Arc<Mutex<Option<EditorState>>>) -> Self {
            Self { state }
        }
    }

    impl EditorStateSource for StubEditorSource {
        fn current(
            &self,
            _cwd: Option<&Utf8Path>,
            _include_dirs: &[Utf8PathBuf],
        ) -> anyhow::Result<Option<EditorState>> {
            Ok(self.state.lock().unwrap().clone())
        }
    }

    // -- Clipboard ------------------------------------------------------------

    /// Stub clipboard source: returns the most recently injected text.
    pub struct StubClipboardSource {
        text: Arc<Mutex<Option<String>>>,
    }

    impl StubClipboardSource {
        pub fn new(text: Arc<Mutex<Option<String>>>) -> Self {
            Self { text }
        }
    }

    impl ClipboardSource for StubClipboardSource {
        fn get_text(&mut self) -> Option<String> {
            self.text.lock().unwrap().clone()
        }

        fn get_image(&mut self) -> Option<ImageData> {
            // Image injection not supported in test mode.
            None
        }
    }

    // -- External selection ---------------------------------------------------

    /// Stub external source: returns the most recently injected snapshot.
    pub struct StubExternalSource {
        snapshot: Arc<Mutex<Option<ExternalSnapshot>>>,
    }

    impl StubExternalSource {
        pub fn new(snapshot: Arc<Mutex<Option<ExternalSnapshot>>>) -> Self {
            Self { snapshot }
        }
    }

    impl ExternalSource for StubExternalSource {
        fn is_available(&self) -> bool {
            true
        }

        fn query(&self) -> Option<ExternalSnapshot> {
            self.snapshot.lock().unwrap().clone()
        }
    }

    // -- Audio ----------------------------------------------------------------

    /// Stub audio source: returns empty chunks (transcription is handled
    /// by `StubTranscriber` which ignores actual audio samples).
    pub struct StubAudioSource {
        sample_rate: u32,
    }

    impl StubAudioSource {
        pub fn new(sample_rate: u32) -> Self {
            Self { sample_rate }
        }
    }

    impl AudioSource for StubAudioSource {
        fn take_chunks(&self) -> Vec<AudioChunk> {
            Vec::new()
        }

        fn sample_rate(&self) -> u32 {
            self.sample_rate
        }

        fn drain(&self) -> Recording {
            Recording { chunks: Vec::new() }
        }

        fn pause(&self) -> anyhow::Result<()> {
            Ok(())
        }

        fn resume(&self) -> anyhow::Result<()> {
            Ok(())
        }

        fn stop(&mut self) -> Recording {
            Recording { chunks: Vec::new() }
        }
    }
}
