mod annotate;
mod parse;

use std::io::IsTerminal;

use camino::{Utf8Path, Utf8PathBuf};

use crate::json::{self, ViewFile, ViewGroup, ViewPayload};
use crate::state::FileEntry;
#[cfg(test)]
use crate::state::Selection;
use crate::state::resolve::relativize;

#[cfg(test)]
use annotate::line_events;
use annotate::{Group, render_line_range};

pub use parse::parse_compact;

/// Cursor marker: U+2758 Light Vertical Bar (non-TTY).
const CURSOR: char = '❘';
/// Selection start marker: U+27E6 Mathematical Left White Square Bracket (non-TTY).
const SEL_OPEN: char = '⟦';
/// Selection end marker: U+27E7 Mathematical Right White Square Bracket (non-TTY).
const SEL_CLOSE: char = '⟧';

/// ANSI escape sequences for TTY color mode.
mod ansi {
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const INVERSE: &str = "\x1b[7m";
    pub const RESET: &str = "\x1b[0m";
}

/// Whether to use ANSI colors or Unicode markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Color,
    Markers,
}

impl Mode {
    /// Pick Color or Markers based on TTY and `NO_COLOR`.
    fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Mode::Markers;
        }
        if std::io::stdout().is_terminal() {
            Mode::Color
        } else {
            Mode::Markers
        }
    }
}

/// How much file content to show around each selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extent {
    /// Only the lines spanned by the selection/cursor.
    Exact,
    /// Additional context lines before/after.
    Lines { before: usize, after: usize },
    /// Entire file contents.
    Full,
}

/// Render file entries with inline content markers or ANSI colors.
///
/// Detects TTY/`NO_COLOR` automatically. Hierarchical output:
/// ```text
/// path
///   selection[, selection...]
///     content with markers/highlights
/// ```
///
/// Overlapping context ranges are merged into a single group with
/// comma-separated position headers.
pub fn render(
    entries: &[FileEntry],
    cwd: Option<&Utf8Path>,
    extent: Extent,
) -> anyhow::Result<String> {
    render_with_mode(entries, cwd, Mode::detect(), extent)
}

/// Inner render with an explicit mode (used by tests to force Markers/Color).
fn render_with_mode(
    entries: &[FileEntry],
    cwd: Option<&Utf8Path>,
    mode: Mode,
    extent: Extent,
) -> anyhow::Result<String> {
    let mut out = String::new();

    for (file_idx, entry) in entries.iter().enumerate() {
        if file_idx > 0 {
            out.push('\n');
        }

        // Resolve path
        let abs_path = if entry.path.is_absolute() {
            entry.path.clone()
        } else {
            let base = match cwd {
                Some(c) => c.to_path_buf(),
                None => Utf8PathBuf::try_from(std::env::current_dir()?).map_err(|e| {
                    anyhow::anyhow!(
                        "non-UTF-8 working directory: {}",
                        e.into_path_buf().display()
                    )
                })?,
            };
            base.join(&entry.path)
        };

        let display_path = relativize(&abs_path, cwd);

        // Read file
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(e) => {
                out.push_str(&format!("{display_path}: {e}\n"));
                continue;
            }
        };
        let lines: Vec<&str> = content.lines().collect();

        // File path header
        if mode == Mode::Color {
            out.push_str(&format!("{}{display_path}{}\n", ansi::BOLD, ansi::RESET));
        } else {
            out.push_str(&format!("{display_path}\n"));
        }

        let groups = Group::compute(&entry.selections, lines.len(), extent);
        let show_headers = extent != Extent::Full;

        for group in &groups {
            if show_headers {
                let header: String = group
                    .sels
                    .iter()
                    .map(|s| s.display_header())
                    .collect::<Vec<_>>()
                    .join(", ");
                if mode == Mode::Color {
                    out.push_str(&format!("  {}{header}{}\n", ansi::DIM, ansi::RESET));
                } else {
                    out.push_str(&format!("  {header}\n"));
                }
            }

            render_line_range(
                &mut out,
                &lines,
                group.first_line,
                group.last_line,
                &group.sels,
                mode,
            );
        }
    }

    Ok(out)
}

/// Build a structured JSON payload for view output.
pub fn render_json(
    entries: &[FileEntry],
    cwd: Option<&Utf8Path>,
    extent: Extent,
) -> anyhow::Result<ViewPayload> {
    let mut files = Vec::new();

    for entry in entries {
        let abs_path = if entry.path.is_absolute() {
            entry.path.clone()
        } else {
            let base = match cwd {
                Some(c) => c.to_path_buf(),
                None => Utf8PathBuf::try_from(std::env::current_dir()?).map_err(|e| {
                    anyhow::anyhow!(
                        "non-UTF-8 working directory: {}",
                        e.into_path_buf().display()
                    )
                })?,
            };
            base.join(&entry.path)
        };

        let display_path = relativize(&abs_path, cwd).to_string();

        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => {
                files.push(ViewFile {
                    path: display_path,
                    groups: Vec::new(),
                });
                continue;
            }
        };
        let lines: Vec<&str> = content.lines().collect();

        let computed = Group::compute(&entry.selections, lines.len(), extent);
        let mut groups = Vec::new();

        for group in &computed {
            let mut rendered = String::new();
            render_line_range(
                &mut rendered,
                &lines,
                group.first_line,
                group.last_line,
                &group.sels,
                Mode::Markers,
            );
            let all_sels: Vec<_> = group.sels.iter().map(|s| **s).collect();
            let (cursors, selections) = json::split_selections(&all_sels);
            groups.push(ViewGroup {
                cursors,
                selections,
                first_line: group.first_line,
                last_line: group.last_line,
                content: strip_indent(&rendered),
            });
        }

        files.push(ViewFile {
            path: display_path,
            groups,
        });
    }

    Ok(ViewPayload { files })
}

/// Strip the 4-space indent that `render_line_range` prepends to every line.
fn strip_indent(s: &str) -> String {
    s.lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
        + if s.ends_with('\n') { "\n" } else { "" }
}

#[cfg(test)]
mod tests;
