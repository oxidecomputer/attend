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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use camino::{Utf8Path, Utf8PathBuf};

use crate::clock::Clock;
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

/// Stub audio source: produces a tiny silent chunk on each poll so the
/// normal transcription pipeline fires. The `StubTranscriber` ignores
/// the actual audio data and returns injected words instead.
///
/// Respects pause/resume: produces nothing while paused, matching
/// the real audio source's behaviour.
pub struct StubAudioSource {
    sample_rate: u32,
    clock: Arc<dyn Clock>,
    paused: Arc<AtomicBool>,
}

impl StubAudioSource {
    pub fn new(sample_rate: u32, clock: Arc<dyn Clock>) -> Self {
        Self {
            sample_rate,
            clock,
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl AudioSource for StubAudioSource {
    fn take_chunks(&self) -> Vec<AudioChunk> {
        if self.paused.load(Ordering::Relaxed) {
            return Vec::new();
        }
        // One sample of silence, timestamped from the mock clock.
        vec![AudioChunk {
            timestamp: self.clock.now(),
            samples: vec![0.0],
        }]
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn drain(&self) -> Recording {
        if self.paused.load(Ordering::Relaxed) {
            return Recording { chunks: Vec::new() };
        }
        Recording {
            chunks: vec![AudioChunk {
                timestamp: self.clock.now(),
                samples: vec![0.0],
            }],
        }
    }

    fn pause(&self) -> anyhow::Result<()> {
        self.paused.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn resume(&self) -> anyhow::Result<()> {
        self.paused.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn stop(&mut self) -> Recording {
        Recording { chunks: Vec::new() }
    }
}
