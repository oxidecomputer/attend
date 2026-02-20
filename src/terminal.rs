//! Terminal helpers: screen management, size queries, and output fitting.

use std::io::{self, Write};

use crossterm::{
    cursor, execute,
    terminal::{self, ClearType},
};

/// Fallback terminal width when size query fails.
const DEFAULT_TERMINAL_WIDTH: usize = 80;

/// Fallback terminal height when size query fails.
const DEFAULT_TERMINAL_HEIGHT: usize = 24;

// ---------------------------------------------------------------------------
// Alternate screen
// ---------------------------------------------------------------------------

/// RAII guard: enters alternate screen on creation, leaves on drop.
pub(crate) struct AlternateScreen;

impl AlternateScreen {
    pub(crate) fn enter() -> Self {
        let _ = execute!(io::stdout(), terminal::EnterAlternateScreen);
        Self
    }
}

impl Drop for AlternateScreen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), terminal::LeaveAlternateScreen);
    }
}

// ---------------------------------------------------------------------------
// Screen operations
// ---------------------------------------------------------------------------

pub(crate) fn clear_screen() {
    let _ = execute!(
        io::stdout(),
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    );
}

pub(crate) fn flush_stdout() {
    io::stdout().flush().ok();
}

/// Query terminal dimensions (columns, rows).
pub(crate) fn terminal_size() -> (usize, usize) {
    terminal::size()
        .map(|(cols, rows)| (cols as usize, rows as usize))
        .unwrap_or((DEFAULT_TERMINAL_WIDTH, DEFAULT_TERMINAL_HEIGHT))
}

/// Truncate a line to `max_cols` visible columns, ANSI-aware.
/// Appends RESET + "…" if truncated.
fn truncate_line(line: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    let mut visible = 0;
    let mut i = 0;
    let bytes = line.as_bytes();
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // Skip ANSI escape sequence (\x1b[...m).
            while i < bytes.len() {
                let b = bytes[i];
                i += 1;
                if b == b'm' {
                    break;
                }
            }
            continue;
        }
        // Count UTF-8 start bytes as visible characters.
        if bytes[i] & 0xC0 != 0x80 {
            visible += 1;
            if visible > max_cols.saturating_sub(1) {
                let mut out = line[..i].to_string();
                out.push_str("\x1b[0m…");
                return out;
            }
        }
        i += 1;
    }
    line.to_string()
}

/// Fit output to terminal dimensions (width + height truncation).
pub(crate) fn fit_to_terminal(output: &str) -> String {
    let (width, height) = terminal_size();
    let mut lines: Vec<String> = output.lines().map(|l| truncate_line(l, width)).collect();
    if lines.len() > height {
        let total = lines.len();
        lines.truncate(height.saturating_sub(1));
        let hidden = total - lines.len();
        lines.push(format!("… {hidden} more lines"));
    }
    lines.join("\n")
}
