//! Check for and deliver pending narration files to Claude Code.
//!
//! Narration files are stored as individual timestamped JSON files in
//! `<cache_dir>/attend/pending/<key>/` where `<key>` is the session ID
//! or `_local` when no agent session was active during recording. Each
//! file contains a `Vec<Event>` with absolute paths. On receive, events
//! are filtered to the current project directory (and any configured
//! `include_dirs`), paths are relativized, and the result is rendered as
//! markdown wrapped in `<narration>` tags.
//!
//! ## Submodules
//!
//! - [`filter`]: Event filtering, path scoping, and redaction markers.
//! - [`pending`]: Pending file collection, archival, and pruning.
//! - [`listen`]: CLI entry points (`run`, `stop`) and lock management.

mod filter;
mod listen;
mod pending;

use std::fs;
use std::path::PathBuf;

use camino::{Utf8Path, Utf8PathBuf};

use super::merge::Event;
use super::render::{self, SnipConfig};

// Re-export the public API so callers still use `receive::*`.
pub use listen::{run, stop};
pub(crate) use pending::{archive_pending, auto_prune, collect_pending};

/// Deserialize, filter, relativize, and render pending JSON event files.
///
/// When `cwd` is `Some`, events are filtered to files under `cwd` or
/// `include_dirs`, and paths are relativized. When `None`, all events
/// pass through unfiltered with absolute paths (used by yank without a
/// session, where there is no project context to filter against).
///
/// Returns `None` if no content remains after filtering.
pub(crate) fn read_pending(
    files: &[PathBuf],
    cwd: Option<&Utf8Path>,
    include_dirs: &[Utf8PathBuf],
    mode: render::RenderMode,
) -> Option<String> {
    if files.is_empty() {
        return None;
    }

    let mut all_events: Vec<Event> = Vec::new();
    for path in files {
        if let Ok(content) = fs::read_to_string(path)
            && let Ok(mut events) = serde_json::from_str::<Vec<Event>>(&content)
        {
            if let Some(cwd) = cwd {
                filter::filter_events(&mut events, cwd, include_dirs);
                filter::relativize_events(&mut events, cwd);
            }
            all_events.append(&mut events);
        }
    }

    if all_events.is_empty() {
        return None;
    }

    // Drop the leading editor snapshot: the UserPromptSubmit hook already
    // delivers the full editor state at delivery time, so the initial
    // snapshot (the state at recording start) is redundant. Done before
    // render so subsequent snapshots (user actions during narration) are
    // preserved.
    if all_events
        .first()
        .is_some_and(|e| matches!(e, Event::EditorSnapshot { .. }))
    {
        all_events.remove(0);
    }

    if all_events.is_empty() {
        return None;
    }

    // Redaction markers alone aren't worth delivering: if everything was
    // filtered and only ✂ markers remain, treat as empty.
    if all_events
        .iter()
        .all(|e| matches!(e, Event::Redacted { .. }))
    {
        return None;
    }

    let markdown = render::render_markdown(&all_events, SnipConfig::default(), mode);
    let trimmed = markdown.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests;
