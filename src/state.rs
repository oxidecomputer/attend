//! Application state: session identity, cache directories, paths, editor
//! state, and compact JSON output.
//!
//! Each concern lives in its own submodule; this file re-exports the public
//! surface so downstream `use crate::state::…` imports continue to work.

mod cache;
mod compact;
mod editor;
mod paths;
mod session_id;

/// Core types (Line, Col, Position, Selection) and byte-offset resolution.
pub(crate) mod resolve;

#[cfg(test)]
mod tests;

// ── Re-exports ──────────────────────────────────────────────────────────────

// session_id
pub use session_id::SessionId;

// cache
#[cfg(test)]
pub(crate) use cache::CacheDirGuard;
pub use cache::cache_dir;

// paths
pub(crate) use paths::{InstallMeta, installed_meta, save_install_meta};
pub use paths::{hooks_dir, listening_path, listening_session};

// resolve (core position types)
pub use resolve::{Col, Line, Position, Selection};

// editor
pub use editor::{EditorState, FileEntry};

// compact
pub use compact::CompactPayload;
