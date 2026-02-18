use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::cli::{Format, Watch, WatchMode};
use crate::state::EditorState;

/// Entry point for the watch subcommand.
pub fn run(watch: &Watch, dir: Option<&Path>) -> anyhow::Result<()> {
    validate_options(watch)?;

    let is_tty = std::io::stdout().is_terminal();

    if watch.mode == WatchMode::Silent {
        eprintln!("attend: watching");
    }

    // Signal handlers: SIGINT for clean shutdown, SIGWINCH for resize.
    let interrupted = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))?;

    let resized = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    signal_hook::flag::register(signal_hook::consts::SIGWINCH, Arc::clone(&resized))?;

    // Alternate screen isolates watch output from scrollback.
    let _screen = if is_tty && watch.mode != WatchMode::Silent {
        Some(AlternateScreen::enter())
    } else {
        None
    };

    run_poll(watch, dir, is_tty, &interrupted, &resized)
}

fn validate_options(watch: &Watch) -> anyhow::Result<()> {
    if watch.mode != WatchMode::View
        && (watch.full || watch.before.is_some() || watch.after.is_some())
    {
        anyhow::bail!("--full, -B, and -A are only valid in view mode");
    }
    if watch.mode != WatchMode::Compact && !matches!(watch.format, Format::Human) {
        anyhow::bail!("--format is only valid in compact mode");
    }
    Ok(())
}

fn compute_extent(watch: &Watch) -> crate::view::Extent {
    if watch.full {
        crate::view::Extent::Full
    } else if watch.before.is_some() || watch.after.is_some() {
        crate::view::Extent::Lines {
            before: watch.before.unwrap_or(0),
            after: watch.after.unwrap_or(0),
        }
    } else {
        crate::view::Extent::Exact
    }
}

// ---------------------------------------------------------------------------
// Alternate screen
// ---------------------------------------------------------------------------

/// RAII guard: enters alternate screen on creation, leaves on drop.
struct AlternateScreen;

impl AlternateScreen {
    fn enter() -> Self {
        print!("\x1b[?1049h");
        flush_stdout();
        Self
    }
}

impl Drop for AlternateScreen {
    fn drop(&mut self) {
        print!("\x1b[?1049l");
        flush_stdout();
    }
}

// ---------------------------------------------------------------------------
// Poll loop
// ---------------------------------------------------------------------------

fn run_poll(
    watch: &Watch,
    dir: Option<&Path>,
    is_tty: bool,
    interrupted: &AtomicBool,
    resized: &AtomicBool,
) -> anyhow::Result<()> {
    let interval = poll_interval(watch);
    let mut prev: Option<EditorState> = None;

    refresh(watch, dir, &mut prev, is_tty, true);

    while !interrupted.load(Ordering::Relaxed) {
        sleep_interruptible(interval, interrupted, resized);
        if interrupted.load(Ordering::Relaxed) {
            break;
        }
        let force = resized.swap(false, Ordering::Relaxed);
        refresh(watch, dir, &mut prev, is_tty, force);
    }

    Ok(())
}

fn sleep_interruptible(duration: Duration, interrupted: &AtomicBool, resized: &AtomicBool) {
    let deadline = std::time::Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero()
            || interrupted.load(Ordering::Relaxed)
            || resized.load(Ordering::Relaxed)
        {
            break;
        }
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

/// Returns `true` if the state changed (or was forced) and output was updated.
fn refresh(
    watch: &Watch,
    dir: Option<&Path>,
    prev: &mut Option<EditorState>,
    is_tty: bool,
    force: bool,
) -> bool {
    let state = match EditorState::current(dir) {
        Ok(s) => s,
        Err(e) => {
            if watch.mode != WatchMode::Silent {
                eprintln!("attend: {e}");
            }
            return false;
        }
    };

    if !force && state.as_ref() == prev.as_ref() {
        return false;
    }

    match watch.mode {
        WatchMode::Silent => {}
        WatchMode::Compact => {
            if let Some(ref s) = state {
                let output = match watch.format {
                    Format::Human => format!("{s}"),
                    Format::Json => {
                        if is_tty {
                            serde_json::to_string_pretty(s).unwrap_or_default()
                        } else {
                            serde_json::to_string(s).unwrap_or_default()
                        }
                    }
                };
                if is_tty {
                    clear_screen();
                    print!("{}", fit_to_terminal(&output));
                } else if matches!(watch.format, Format::Json) {
                    // JSON-lines: one compact object per change.
                    println!("{output}");
                } else {
                    print!("{output}\n\n");
                }
            } else if is_tty {
                clear_screen();
            }
        }
        WatchMode::View => {
            if let Some(ref s) = state {
                let extent = compute_extent(watch);
                match crate::view::render(&s.files, dir, extent) {
                    Ok(output) => {
                        if is_tty {
                            clear_screen();
                            print!("{}", fit_to_terminal(&output));
                        } else {
                            print!("{output}\n\n");
                        }
                    }
                    Err(e) => eprintln!("attend: {e}"),
                }
            } else if is_tty {
                clear_screen();
            }
        }
    }

    flush_stdout();
    *prev = state;
    true
}

// ---------------------------------------------------------------------------
// Terminal helpers
// ---------------------------------------------------------------------------

fn clear_screen() {
    print!("\x1b[2J\x1b[H");
}

fn flush_stdout() {
    std::io::stdout().flush().ok();
}

/// Query terminal dimensions (columns, rows).
fn terminal_size() -> (usize, usize) {
    #[cfg(unix)]
    {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0
            && ws.ws_row > 0
            && ws.ws_col > 0
        {
            return (ws.ws_col as usize, ws.ws_row as usize);
        }
    }
    (80, 24)
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
fn fit_to_terminal(output: &str) -> String {
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

// ---------------------------------------------------------------------------
// Duration helpers
// ---------------------------------------------------------------------------

fn poll_interval(watch: &Watch) -> Duration {
    if let Some(secs) = watch.interval {
        return Duration::from_secs_f64(secs);
    }
    match watch.mode {
        WatchMode::Silent => Duration::from_secs(5),
        _ => Duration::from_millis(100),
    }
}
