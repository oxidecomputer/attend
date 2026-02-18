//! Recording daemon: captures audio, editor state, and file diffs.
//!
//! The recorder spawns as a detached child process via `_record-daemon`.
//! The parent (toggle/start) exits immediately so the hotkey returns fast.
//! The daemon records until a stop sentinel file appears, then transcribes,
//! merges all streams, and writes the result as a pending dictation file.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use super::audio;
use super::merge::{self, Event, RenderedFile};
use super::transcribe;
use super::{
    cache_dir, default_model_path, pending_dir, record_lock_path, resolve_session,
    stop_sentinel_path,
};
use crate::json::utc_now;
use crate::state::{self, EditorState};
use crate::view;

/// Toggle recording: start if not recording, stop if recording.
pub fn toggle(
    model: Option<PathBuf>,
    session: Option<String>,
    snip_cfg: merge::SnipConfig,
) -> anyhow::Result<()> {
    if record_lock_path().exists() {
        stop()
    } else {
        start(model, session, snip_cfg)
    }
}

/// Start recording by spawning a detached daemon process.
///
/// If already recording (lock exists), this is a no-op.
pub fn start(
    model: Option<PathBuf>,
    session: Option<String>,
    snip_cfg: merge::SnipConfig,
) -> anyhow::Result<()> {
    if record_lock_path().exists() {
        eprintln!("Already recording.");
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("dictate").arg("_record-daemon");

    if let Some(ref m) = model {
        cmd.arg("--model").arg(m);
    }
    if let Some(ref s) = session {
        cmd.arg("--session").arg(s);
    }

    let defaults = merge::SnipConfig::default();
    if snip_cfg.threshold != defaults.threshold {
        cmd.arg("--snip-threshold")
            .arg(snip_cfg.threshold.to_string());
    }
    if snip_cfg.head != defaults.head {
        cmd.arg("--snip-head").arg(snip_cfg.head.to_string());
    }
    if snip_cfg.tail != defaults.tail {
        cmd.arg("--snip-tail").arg(snip_cfg.tail.to_string());
    }

    // Detach: redirect stdio to /dev/null
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit()); // keep stderr for debugging

    cmd.spawn()?;

    // Give the daemon a moment to acquire the lock and start audio
    thread::sleep(Duration::from_millis(200));

    Ok(())
}

/// Signal the recorder to stop by creating the stop sentinel.
///
/// If not recording (no lock), this is a no-op.
pub fn stop() -> anyhow::Result<()> {
    if !record_lock_path().exists() {
        eprintln!("Not recording.");
        return Ok(());
    }

    let sentinel = stop_sentinel_path();
    if let Some(parent) = sentinel.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&sentinel, "")?;

    // Wait briefly for the daemon to notice
    for _ in 0..100 {
        if !record_lock_path().exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }

    eprintln!("Stop signal sent; daemon may still be transcribing.");
    Ok(())
}

/// The actual recording daemon entry point.
///
/// Acquires the record lock, captures audio + editor state + file diffs,
/// waits for the stop sentinel, transcribes, merges, and writes output.
pub fn daemon(
    model: Option<PathBuf>,
    session: Option<String>,
    snip_cfg: merge::SnipConfig,
) -> anyhow::Result<()> {
    let model_path = model.unwrap_or_else(default_model_path);
    let session_id = resolve_session(session);

    // Ensure cache dir exists
    let cd = cache_dir();
    fs::create_dir_all(&cd)?;

    // Acquire record lock
    let lock_path = record_lock_path();
    if lock_path.exists() {
        anyhow::bail!("record lock already held");
    }
    fs::write(&lock_path, std::process::id().to_string())?;

    // Clean up any stale stop sentinel
    let _ = fs::remove_file(stop_sentinel_path());

    // Play start chime
    let _ = audio::play_chime(true);

    // Start audio capture
    let capture = audio::start_capture()?;

    // Use current working directory for relative path display
    let cwd = std::env::current_dir().ok();

    // Start editor polling and file diff tracking on background threads
    let editor_events = poll_editor_and_diffs(cwd)?;

    // Wait for stop sentinel
    let sentinel = stop_sentinel_path();
    while !sentinel.exists() {
        thread::sleep(Duration::from_millis(100));
    }

    // Play stop chime
    let _ = audio::play_chime(false);

    // Stop audio capture
    let recording = capture.stop();

    // Clean up sentinel
    let _ = fs::remove_file(&sentinel);

    // Bail early if recording is too short
    if recording.duration_secs() < 0.5 {
        eprintln!(
            "Recording too short ({:.1}s), discarding.",
            recording.duration_secs()
        );
        let _ = fs::remove_file(&lock_path);
        return Ok(());
    }

    eprintln!("Transcribing {:.1}s of audio...", recording.duration_secs());

    // Ensure model exists (downloads if needed)
    transcribe::ensure_model(&model_path)?;

    // Resample to 16kHz
    let samples_16k = audio::resample(&recording.flatten(), recording.sample_rate, 16000)?;

    // Transcribe
    let words = transcribe::transcribe(&samples_16k, &model_path)?;

    // Collect editor events
    let (editor_snapshots, file_diffs) = editor_events.collect();

    // Build merged event list
    let mut events: Vec<Event> = Vec::new();

    // Add transcribed words
    for word in &words {
        events.push(Event::Words {
            offset_secs: word.start_secs,
            text: word.text.clone(),
        });
    }

    // Add editor snapshots
    events.extend(editor_snapshots);

    // Add file diffs
    events.extend(file_diffs);

    // Format as markdown
    let markdown = merge::format_markdown(&mut events, snip_cfg);

    if markdown.trim().is_empty() {
        eprintln!("No content captured, discarding.");
        let _ = fs::remove_file(&lock_path);
        return Ok(());
    }

    // Write to pending directory with timestamp filename
    let ts = utc_now().replace(':', "-"); // filesystem-safe timestamp
    if let Some(sid) = &session_id {
        let dir = pending_dir(sid);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{ts}.md"));
        fs::write(&path, &markdown)?;
        eprintln!("Dictation written to {}", path.display());
    } else {
        // Fallback: unsuffixed file
        let path = cache_dir().join("dictation.md");
        fs::write(&path, &markdown)?;
        eprintln!("Dictation written to {}", path.display());
    }

    // Release lock
    let _ = fs::remove_file(&lock_path);

    Ok(())
}

/// Handle for the background editor/diff polling threads.
struct EditorPollHandle {
    stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    editor_thread: Option<thread::JoinHandle<Vec<Event>>>,
    diff_thread: Option<thread::JoinHandle<Vec<Event>>>,
}

impl EditorPollHandle {
    /// Signal stop and collect results.
    fn collect(mut self) -> (Vec<Event>, Vec<Event>) {
        self.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let editor_events = self
            .editor_thread
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        let diff_events = self
            .diff_thread
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();

        (editor_events, diff_events)
    }
}

/// Start background threads for editor polling and file diff tracking.
fn poll_editor_and_diffs(cwd: Option<PathBuf>) -> anyhow::Result<EditorPollHandle> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let stop_flag = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    // Editor state polling thread
    let stop_ed = Arc::clone(&stop_flag);
    let ed_cwd = cwd.clone();
    let editor_thread = thread::spawn(move || {
        let mut events = Vec::new();
        let mut prev_files: Option<Vec<state::FileEntry>> = None;
        let mut is_first = true;

        while !stop_ed.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));

            let state = match EditorState::current(ed_cwd.as_deref()) {
                Ok(Some(s)) => s,
                _ => continue,
            };

            // Check if file entries changed
            if prev_files.as_ref() == Some(&state.files) {
                continue;
            }

            let files = state.files;
            prev_files = Some(files.clone());

            // Suppress the initial snapshot (cursor position before user navigates)
            if is_first {
                is_first = false;
                continue;
            }

            let offset_secs = start.elapsed().as_secs_f64();
            let rendered = render_snapshot_files(&files, state.cwd.as_deref());

            events.push(Event::EditorSnapshot {
                offset_secs,
                files,
                rendered,
            });
        }

        events
    });

    // File diff tracking thread
    let stop_diff = Arc::clone(&stop_flag);
    let diff_cwd = cwd;
    let diff_thread = thread::spawn(move || {
        let mut events = Vec::new();
        let mut file_contents: HashMap<PathBuf, String> = HashMap::new();
        let mut file_mtimes: HashMap<PathBuf, std::time::SystemTime> = HashMap::new();

        // Snapshot initial state of recently active files
        if let Ok(Some(state)) = EditorState::current(diff_cwd.as_deref()) {
            for file in &state.files {
                if let Ok(content) = fs::read_to_string(&file.path) {
                    if let Ok(meta) = fs::metadata(&file.path)
                        && let Ok(mtime) = meta.modified()
                    {
                        file_mtimes.insert(file.path.clone(), mtime);
                    }
                    file_contents.insert(file.path.clone(), content);
                }
            }
        }

        while !stop_diff.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));

            // Check current editor files for changes
            let state = match EditorState::current(diff_cwd.as_deref()) {
                Ok(Some(s)) => s,
                _ => continue,
            };

            for file in &state.files {
                let Ok(meta) = fs::metadata(&file.path) else {
                    continue;
                };
                let Ok(mtime) = meta.modified() else {
                    continue;
                };

                let changed = file_mtimes
                    .get(&file.path)
                    .map(|prev| *prev != mtime)
                    .unwrap_or(true);

                if !changed {
                    continue;
                }

                file_mtimes.insert(file.path.clone(), mtime);

                let Ok(new_content) = fs::read_to_string(&file.path) else {
                    continue;
                };

                if let Some(old_content) = file_contents.get(&file.path)
                    && *old_content != new_content
                {
                    let offset_secs = start.elapsed().as_secs_f64();
                    let diff = merge::unified_diff(old_content, &new_content);
                    if !diff.trim().is_empty() {
                        let display_path = relativize_path(&file.path, diff_cwd.as_deref());
                        events.push(Event::FileDiff {
                            offset_secs,
                            path: display_path,
                            diff,
                        });
                    }
                }

                file_contents.insert(file.path.clone(), new_content);
            }
        }

        events
    });

    Ok(EditorPollHandle {
        stop_flag,
        editor_thread: Some(editor_thread),
        diff_thread: Some(diff_thread),
    })
}

/// Render file entries into `RenderedFile` entries with relative paths.
fn render_snapshot_files(files: &[state::FileEntry], cwd: Option<&Path>) -> Vec<RenderedFile> {
    let mut rendered = Vec::new();

    for file in files {
        if file.selections.is_empty() {
            continue;
        }

        let Ok(payload) = view::render_json(std::slice::from_ref(file), cwd, view::Extent::Exact)
        else {
            continue;
        };

        for vf in payload.files {
            for group in &vf.groups {
                rendered.push(RenderedFile {
                    path: vf.path.clone(),
                    content: group.content.clone(),
                    first_line: group.first_line.get() as u32,
                });
            }
        }
    }

    rendered
}

/// Relativize a path against a working directory for display.
fn relativize_path(path: &Path, cwd: Option<&Path>) -> String {
    if let Some(base) = cwd
        && let Ok(rel) = path.strip_prefix(base)
    {
        return rel.to_string_lossy().to_string();
    }
    path.to_string_lossy().to_string()
}
