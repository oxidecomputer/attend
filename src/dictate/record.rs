//! Recording daemon: captures audio, editor state, and file diffs.
//!
//! The recorder spawns as a detached child process via `_record-daemon`.
//! The parent (toggle/start) exits immediately so the hotkey returns fast.
//! The daemon records until a stop sentinel file appears, then transcribes,
//! merges all streams, and writes the result as a pending dictation file.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use super::audio;
use super::merge::{self, Event, RenderedFile};
use super::transcribe::Engine;
use super::{
    cache_dir, flush_sentinel_path, pending_dir, record_lock_path, resolve_session,
    stop_sentinel_path,
};
use crate::json::utc_now;
use crate::state::{self, EditorState};
use crate::view;

/// Toggle recording: start if not recording, stop if recording.
pub fn toggle(
    engine: Engine,
    model: Option<PathBuf>,
    session: Option<String>,
    snip_cfg: merge::SnipConfig,
) -> anyhow::Result<()> {
    let lock = record_lock_path();
    if lock.exists() {
        // Check for stale lock (daemon was killed without cleanup).
        if is_lock_stale(&lock) {
            eprintln!("Stale record lock detected, cleaning up.");
            let _ = fs::remove_file(&lock);
            let _ = fs::remove_file(stop_sentinel_path());
            start(engine, model, session, snip_cfg)
        } else {
            stop()
        }
    } else {
        start(engine, model, session, snip_cfg)
    }
}

/// Check whether a lock file is stale (the owning process is no longer alive).
fn is_lock_stale(lock_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(lock_path) else {
        return false;
    };
    let Ok(pid) = content.trim().parse::<i32>() else {
        // No PID in the file — can't determine, assume not stale.
        return false;
    };
    // kill(pid, 0) checks if the process exists without sending a signal.
    unsafe { libc::kill(pid, 0) != 0 }
}

/// Start recording by spawning a detached daemon process.
///
/// If already recording (lock exists), this is a no-op.
pub fn start(
    engine: Engine,
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

    // Forward engine selection to daemon
    let engine_str = match engine {
        Engine::Whisper => "whisper",
        Engine::Parakeet => "parakeet",
    };
    cmd.arg("--engine").arg(engine_str);

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

    // Detach: redirect all stdio to /dev/null and start a new session
    // so the daemon survives if the parent's process group is killed
    // (e.g. when Zed's task runner cleans up after toggle exits).
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid() is async-signal-safe and has no preconditions.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

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

/// Flush: submit current dictation and keep recording.
///
/// If not recording (no lock), starts recording (like toggle).
/// If recording, creates the flush sentinel and waits for the daemon to
/// acknowledge it (by deleting the sentinel).
pub fn flush(
    engine: Engine,
    model: Option<PathBuf>,
    session: Option<String>,
    snip_cfg: merge::SnipConfig,
) -> anyhow::Result<()> {
    let lock = record_lock_path();
    if !lock.exists() {
        // Not recording — start.
        return start(engine, model, session, snip_cfg);
    }

    if is_lock_stale(&lock) {
        eprintln!("Stale record lock detected, cleaning up.");
        let _ = fs::remove_file(&lock);
        let _ = fs::remove_file(stop_sentinel_path());
        let _ = fs::remove_file(flush_sentinel_path());
        return start(engine, model, session, snip_cfg);
    }

    // Recording — create flush sentinel.
    let sentinel = flush_sentinel_path();
    if let Some(parent) = sentinel.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&sentinel, "")?;

    // Wait for the daemon to delete the sentinel (acknowledging the flush).
    for _ in 0..100 {
        if !sentinel.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }

    eprintln!("Flush signal sent; daemon may still be transcribing.");
    Ok(())
}

/// The actual recording daemon entry point.
///
/// Acquires the record lock, captures audio + editor state + file diffs,
/// waits for stop/flush sentinels, transcribes, merges, and writes output.
pub fn daemon(
    engine: Engine,
    model: Option<PathBuf>,
    session: Option<String>,
    snip_cfg: merge::SnipConfig,
) -> anyhow::Result<()> {
    let model_path = model.unwrap_or_else(|| engine.default_model_path());
    let session_id = resolve_session(session);

    // Ensure cache dir exists
    let cd = cache_dir();
    fs::create_dir_all(&cd)?;

    // Acquire record lock (auto-removed on drop, even on error/panic)
    let _lock = lockfile::Lockfile::create(record_lock_path())
        .map_err(|e| anyhow::anyhow!("record lock already held: {e:?}"))?;

    // Write our PID so the lock can be detected as stale if we're killed
    let _ = fs::write(record_lock_path(), std::process::id().to_string());

    // Clean up any stale sentinels
    let _ = fs::remove_file(stop_sentinel_path());
    let _ = fs::remove_file(flush_sentinel_path());

    // Preload model (blocks until ready — must complete before first transcription)
    let mut transcriber = engine.preload(&model_path)?;

    // Play start chime
    let _ = audio::play_chime(true);

    // Start audio capture
    let capture = audio::start_capture()?;

    // Use current working directory for relative path display
    let cwd = std::env::current_dir().ok();

    // Start editor polling and file diff tracking on background threads
    let editor_events = poll_editor_and_diffs(cwd)?;

    // Track time base for word timestamp offsets across flushes
    let mut time_base_secs = 0.0_f64;
    let mut last_drain = Instant::now();

    let stop_sentinel = stop_sentinel_path();
    let flush_sentinel = flush_sentinel_path();

    loop {
        // Check for stop sentinel
        if stop_sentinel.exists() {
            let _ = audio::play_chime(false);
            let recording = capture.stop();
            let _ = fs::remove_file(&stop_sentinel);

            let (editor_snapshots, file_diffs) = editor_events.collect();
            transcribe_and_write(
                &mut *transcriber,
                recording,
                editor_snapshots,
                file_diffs,
                time_base_secs,
                &session_id,
                snip_cfg,
            )?;
            break;
        }

        // Check for flush sentinel
        if flush_sentinel.exists() {
            let _ = audio::play_flush_chime();
            let recording = capture.drain();
            let elapsed = last_drain.elapsed().as_secs_f64();

            let (editor_snapshots, file_diffs) = editor_events.drain();
            transcribe_and_write(
                &mut *transcriber,
                recording,
                editor_snapshots,
                file_diffs,
                time_base_secs,
                &session_id,
                snip_cfg,
            )?;

            time_base_secs += elapsed;
            last_drain = Instant::now();

            // Acknowledge flush by deleting sentinel
            let _ = fs::remove_file(&flush_sentinel);
            continue;
        }

        thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}

/// Transcribe audio, merge with editor events, and write the pending file.
fn transcribe_and_write(
    transcriber: &mut dyn super::transcribe::Transcriber,
    recording: audio::Recording,
    editor_snapshots: Vec<Event>,
    file_diffs: Vec<Event>,
    time_base_secs: f64,
    session_id: &Option<String>,
    snip_cfg: merge::SnipConfig,
) -> anyhow::Result<()> {
    if recording.duration_secs() < 0.5 {
        eprintln!(
            "Recording too short ({:.1}s), discarding.",
            recording.duration_secs()
        );
        return Ok(());
    }

    eprintln!("Transcribing {:.1}s of audio...", recording.duration_secs());

    let samples_16k = audio::resample(&recording.flatten(), recording.sample_rate, 16000)?;
    let words = transcriber.transcribe(&samples_16k)?;

    let mut events: Vec<Event> = Vec::new();

    for word in &words {
        events.push(Event::Words {
            offset_secs: word.start_secs + time_base_secs,
            text: word.text.clone(),
        });
    }

    events.extend(editor_snapshots);
    events.extend(file_diffs);

    let markdown = merge::format_markdown(&mut events, snip_cfg);

    if markdown.trim().is_empty() {
        eprintln!("No content captured, discarding.");
        return Ok(());
    }

    let ts = utc_now().replace(':', "-");
    if let Some(sid) = session_id {
        let dir = pending_dir(sid);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{ts}.md"));
        fs::write(&path, &markdown)?;
        eprintln!("Dictation written to {}", path.display());
    } else {
        let path = cache_dir().join("dictation.md");
        fs::write(&path, &markdown)?;
        eprintln!("Dictation written to {}", path.display());
    }

    Ok(())
}

/// Handle for the background editor/diff polling threads.
struct EditorPollHandle {
    stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// When set, the editor thread resets prev_files so the current cursor
    /// position is re-emitted as context for the next batch.
    reset_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    editor_events: std::sync::Arc<Mutex<Vec<Event>>>,
    diff_events: std::sync::Arc<Mutex<Vec<Event>>>,
    editor_thread: Option<thread::JoinHandle<()>>,
    diff_thread: Option<thread::JoinHandle<()>>,
}

impl EditorPollHandle {
    /// Drain accumulated events without stopping threads.
    ///
    /// Signals the editor thread to re-emit the current cursor position,
    /// so the next batch includes editor context even if the user hasn't
    /// moved since the last drain.
    fn drain(&self) -> (Vec<Event>, Vec<Event>) {
        let editor = std::mem::take(&mut *self.editor_events.lock().unwrap());
        let diff = std::mem::take(&mut *self.diff_events.lock().unwrap());
        self.reset_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        (editor, diff)
    }

    /// Signal stop and collect remaining results.
    fn collect(mut self) -> (Vec<Event>, Vec<Event>) {
        self.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);

        if let Some(h) = self.editor_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.diff_thread.take() {
            let _ = h.join();
        }

        self.drain()
    }
}

/// Start background threads for editor polling and file diff tracking.
fn poll_editor_and_diffs(cwd: Option<PathBuf>) -> anyhow::Result<EditorPollHandle> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let stop_flag = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    let editor_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let diff_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));

    let reset_flag = Arc::new(AtomicBool::new(false));

    // Editor state polling thread
    let stop_ed = Arc::clone(&stop_flag);
    let reset_ed = Arc::clone(&reset_flag);
    let ed_cwd = cwd.clone();
    let ed_events = Arc::clone(&editor_events);
    let editor_thread = thread::spawn(move || {
        let mut prev_files: Option<Vec<state::FileEntry>> = None;

        while !stop_ed.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));

            // After a flush, reset so we re-capture the current cursor
            // position as context for the next batch.
            if reset_ed.swap(false, Ordering::Relaxed) {
                prev_files = None;
            }

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

            let offset_secs = start.elapsed().as_secs_f64();
            let rendered = render_snapshot_files(&files, state.cwd.as_deref());

            ed_events.lock().unwrap().push(Event::EditorSnapshot {
                offset_secs,
                files,
                rendered,
            });
        }
    });

    // File diff tracking thread
    let stop_diff = Arc::clone(&stop_flag);
    let diff_cwd = cwd;
    let df_events = Arc::clone(&diff_events);
    let diff_thread = thread::spawn(move || {
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
                    let display_path = relativize_path(&file.path, diff_cwd.as_deref());
                    df_events.lock().unwrap().push(Event::FileDiff {
                        offset_secs,
                        path: display_path,
                        old: old_content.clone(),
                        new: new_content.clone(),
                    });
                }

                file_contents.insert(file.path.clone(), new_content);
            }
        }
    });

    Ok(EditorPollHandle {
        stop_flag,
        reset_flag,
        editor_events,
        diff_events,
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
