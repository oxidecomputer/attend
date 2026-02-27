//! Wire protocol types for the inject socket.
//!
//! These types mirror the definitions in `attend::test_mode` and
//! `attend::state`. They must serialize to identical JSON. The harness
//! serializes `Inject` messages; it deserializes `Handshake` responses.

use std::num::NonZeroUsize;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Inject messages (harness → process)
// ---------------------------------------------------------------------------

/// Injection message broadcast from the harness to all connected processes.
///
/// Mirrors `attend::test_mode::Inject`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Inject {
    /// Advance the mock clock by this duration.
    AdvanceTime { duration_ms: u64 },
    /// Inject speech into the stub transcriber.
    Speech { text: String, duration_ms: u64 },
    /// Inject a period of silence.
    Silence { duration_ms: u64 },
    /// Set the stub editor state.
    EditorState { files: Vec<FileEntry> },
    /// Set the stub external selection.
    ExternalSelection { app: String, text: String },
    /// Set the stub clipboard content.
    Clipboard { text: String },
}

// ---------------------------------------------------------------------------
// Handshake (process → harness)
// ---------------------------------------------------------------------------

/// Handshake sent by each process on connecting to the inject socket.
///
/// Mirrors `attend::test_mode::Handshake`.
#[derive(Debug, Deserialize)]
pub struct Handshake {
    pub pid: u32,
    pub argv: Vec<String>,
}

// ---------------------------------------------------------------------------
// State types (mirrored from attend::state)
// ---------------------------------------------------------------------------

/// An open file with cursor/selection positions.
///
/// Mirrors `attend::state::FileEntry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: Utf8PathBuf,
    pub selections: Vec<Selection>,
}

/// A cursor position or text selection range.
///
/// Mirrors `attend::state::resolve::Selection`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection {
    pub start: Position,
    pub end: Position,
}

/// A 1-based line:col position in a file.
///
/// Mirrors `attend::state::resolve::Position`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub line: Line,
    pub col: Col,
}

/// 1-based line number. Serializes as a plain integer via `#[serde(transparent)]`.
///
/// Mirrors `attend::state::resolve::Line`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Line(NonZeroUsize);

impl Line {
    /// Create a line number. Panics if `n` is 0.
    pub fn new(n: usize) -> Self {
        Self(NonZeroUsize::new(n).expect("line number must be >= 1"))
    }
}

/// 1-based column number. Serializes as a plain integer via `#[serde(transparent)]`.
///
/// Mirrors `attend::state::resolve::Col`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Col(NonZeroUsize);

impl Col {
    /// Create a column number. Panics if `n` is 0.
    pub fn new(n: usize) -> Self {
        Self(NonZeroUsize::new(n).expect("column number must be >= 1"))
    }
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

impl FileEntry {
    /// Create a file entry with no selections (just the file path).
    pub fn path_only(path: impl Into<Utf8PathBuf>) -> Self {
        Self {
            path: path.into(),
            selections: Vec::new(),
        }
    }

    /// Create a file entry with a single cursor position.
    pub fn with_cursor(path: impl Into<Utf8PathBuf>, line: usize, col: usize) -> Self {
        let pos = Position {
            line: Line::new(line),
            col: Col::new(col),
        };
        Self {
            path: path.into(),
            selections: vec![Selection {
                start: pos.clone(),
                end: pos,
            }],
        }
    }
}
