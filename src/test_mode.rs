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

use serde::{Deserialize, Serialize};

use crate::state::FileEntry;

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
    //! Channel-backed stub implementations of capture source traits.
    //!
    //! Each stub holds shared state (`Arc<Mutex<...>>`) that the inject
    //! router writes to and the capture thread reads from. This decouples
    //! the inject socket background thread from the capture polling loops.

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
