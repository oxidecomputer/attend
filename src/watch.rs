use std::io::IsTerminal;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use camino::Utf8Path;

use crate::cli::Format;
use crate::state::EditorState;
use crate::terminal::{AlternateScreen, clear_screen, fit_to_terminal, flush_stdout};

/// Default poll interval for silent (daemon) mode (secs).
const WATCH_SILENT_POLL_SECS: u64 = 5;

/// Default poll interval for live display modes (ms).
const WATCH_LIVE_POLL_MS: u64 = 100;

/// Granularity of the interruptible sleep loop (ms).
const SLEEP_GRANULARITY_MS: u64 = 50;

/// Display mode for the watch loop.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WatchMode {
    /// Daemon: continuously update cache, no output.
    Silent,
    /// Live compact output (paths + positions).
    Compact,
    /// Live view output (file content with markers).
    View,
}

/// Entry point for the watch loop (used by Glance --watch, Look --watch, and Meditate).
pub fn run(
    mode: WatchMode,
    dir: Option<&Utf8Path>,
    interval: Option<f64>,
    format: &Format,
    full: bool,
    before: Option<usize>,
    after: Option<usize>,
) -> anyhow::Result<()> {
    validate_options(mode, format, full, before, after)?;

    let is_tty = std::io::stdout().is_terminal();

    if mode == WatchMode::Silent {
        tracing::info!("watching");
    }

    // Signal handlers: SIGINT for clean shutdown, SIGWINCH for resize.
    let interrupted = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))?;

    let resized = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGWINCH, Arc::clone(&resized))?;

    // Alternate screen isolates watch output from scrollback.
    let _screen = if is_tty && mode != WatchMode::Silent {
        Some(AlternateScreen::enter())
    } else {
        None
    };

    run_poll(
        mode,
        dir,
        interval,
        format,
        full,
        before,
        after,
        is_tty,
        &interrupted,
        &resized,
    )
}

fn validate_options(
    mode: WatchMode,
    format: &Format,
    full: bool,
    before: Option<usize>,
    after: Option<usize>,
) -> anyhow::Result<()> {
    if mode != WatchMode::View && (full || before.is_some() || after.is_some()) {
        anyhow::bail!("--full, -B, and -A are only valid in view mode");
    }
    if mode == WatchMode::Silent && !matches!(format, Format::Human) {
        anyhow::bail!("--format is not valid in silent mode");
    }
    Ok(())
}

fn compute_extent(full: bool, before: Option<usize>, after: Option<usize>) -> crate::view::Extent {
    if full {
        crate::view::Extent::Full
    } else if before.is_some() || after.is_some() {
        crate::view::Extent::Lines {
            before: before.unwrap_or(0),
            after: after.unwrap_or(0),
        }
    } else {
        crate::view::Extent::Exact
    }
}

// ---------------------------------------------------------------------------
// Poll loop
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_poll(
    mode: WatchMode,
    dir: Option<&Utf8Path>,
    interval: Option<f64>,
    format: &Format,
    full: bool,
    before: Option<usize>,
    after: Option<usize>,
    is_tty: bool,
    interrupted: &AtomicBool,
    resized: &AtomicBool,
) -> anyhow::Result<()> {
    let poll_dur = poll_interval(mode, interval);
    let mut prev: Option<EditorState> = None;

    refresh(
        mode, dir, format, full, before, after, &mut prev, is_tty, true,
    );

    while !interrupted.load(Ordering::Relaxed) {
        sleep_interruptible(poll_dur, interrupted, resized);
        if interrupted.load(Ordering::Relaxed) {
            break;
        }
        let force = resized.swap(false, Ordering::Relaxed);
        refresh(
            mode, dir, format, full, before, after, &mut prev, is_tty, force,
        );
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
        thread::sleep(remaining.min(Duration::from_millis(SLEEP_GRANULARITY_MS)));
    }
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

/// Returns `true` if the state changed (or was forced) and output was updated.
#[allow(clippy::too_many_arguments)]
fn refresh(
    mode: WatchMode,
    dir: Option<&Utf8Path>,
    format: &Format,
    full: bool,
    before: Option<usize>,
    after: Option<usize>,
    prev: &mut Option<EditorState>,
    is_tty: bool,
    force: bool,
) -> bool {
    let state = match EditorState::current(dir, &[]) {
        Ok(s) => s,
        Err(e) => {
            if mode != WatchMode::Silent {
                tracing::warn!("{e}");
            }
            return false;
        }
    };

    if !force && state.as_ref() == prev.as_ref() {
        return false;
    }

    match mode {
        WatchMode::Silent => {}
        WatchMode::Compact => {
            if let Some(ref s) = state {
                let output = match format {
                    Format::Human => format!("{s}"),
                    Format::Json => {
                        let payload = crate::state::CompactPayload::from_state(s);
                        let wrapped = crate::util::Timestamped::now(payload);
                        if is_tty {
                            serde_json::to_string_pretty(&wrapped)
                                .expect("serialization of known type")
                        } else {
                            serde_json::to_string(&wrapped).expect("serialization of known type")
                        }
                    }
                };
                if is_tty {
                    clear_screen();
                    print!("{}", fit_to_terminal(&output));
                } else if matches!(format, Format::Json) {
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
                let extent = compute_extent(full, before, after);
                match format {
                    Format::Human => match crate::view::render(&s.files, dir, extent) {
                        Ok(output) => {
                            if is_tty {
                                clear_screen();
                                print!("{}", fit_to_terminal(&output));
                            } else {
                                print!("{output}\n\n");
                            }
                        }
                        Err(e) => eprintln!("attend: {e}"),
                    },
                    Format::Json => match crate::view::render_json(&s.files, dir, extent) {
                        Ok(payload) => {
                            let wrapped = crate::util::Timestamped::now(payload);
                            let output = if is_tty {
                                serde_json::to_string_pretty(&wrapped)
                                    .expect("serialization of known type")
                            } else {
                                serde_json::to_string(&wrapped)
                                    .expect("serialization of known type")
                            };
                            if is_tty {
                                clear_screen();
                                print!("{}", fit_to_terminal(&output));
                            } else {
                                println!("{output}");
                            }
                        }
                        Err(e) => eprintln!("attend: {e}"),
                    },
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
// Duration helpers
// ---------------------------------------------------------------------------

fn poll_interval(mode: WatchMode, interval: Option<f64>) -> Duration {
    if let Some(secs) = interval {
        return Duration::from_secs_f64(secs);
    }
    match mode {
        WatchMode::Silent => Duration::from_secs(WATCH_SILENT_POLL_SECS),
        _ => Duration::from_millis(WATCH_LIVE_POLL_MS),
    }
}
