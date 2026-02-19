//! Recording daemon: captures audio, editor state, and file diffs.
//!
//! The recorder spawns as a detached child process via `_record-daemon`.
//! The parent (toggle/start) exits immediately so the hotkey returns fast.
//! The daemon records until a stop sentinel file appears, then transcribes,
//! merges all streams, and writes the result as a pending narration file.

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use super::audio;
use super::capture;
use super::merge::{self, Event};
use super::transcribe::Engine;
use super::{
    cache_dir, flush_sentinel_path, pending_dir, record_lock_path, resolve_session,
    stop_sentinel_path,
};
use crate::config::Config;
use crate::json::utc_now;

/// Toggle recording: start if not recording, stop if recording.
pub fn toggle() -> anyhow::Result<()> {
    let lock = record_lock_path();
    if lock.exists() {
        // Check for stale lock (daemon was killed without cleanup).
        if is_lock_stale(&lock) {
            tracing::warn!("Stale record lock detected, cleaning up.");
            let _ = fs::remove_file(&lock);
            let _ = fs::remove_file(stop_sentinel_path());
            spawn_daemon()
        } else {
            stop()
        }
    } else {
        spawn_daemon()
    }
}

/// Check whether a lock file is stale (the owning process is no longer alive).
pub(crate) fn is_lock_stale(lock_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(lock_path) else {
        return false;
    };
    let Ok(pid) = content.trim().parse::<i32>() else {
        // No PID in the file — can't determine, assume not stale.
        return false;
    };
    !super::process_alive(pid)
}

/// Start recording, or flush current narration and keep recording.
///
/// If not recording, spawns a new daemon. If already recording, signals
/// the daemon to flush (submit current narration and continue).
pub fn start() -> anyhow::Result<()> {
    let lock = record_lock_path();

    // Already recording — flush instead.
    if lock.exists() {
        if is_lock_stale(&lock) {
            tracing::warn!("Stale record lock detected, cleaning up.");
            let _ = fs::remove_file(&lock);
            let _ = fs::remove_file(stop_sentinel_path());
            let _ = fs::remove_file(flush_sentinel_path());
            // Fall through to spawn below.
        } else {
            return flush();
        }
    }

    spawn_daemon()
}

/// Signal the running daemon to flush (submit current narration, keep recording).
fn flush() -> anyhow::Result<()> {
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

/// Spawn a detached recording daemon process.
fn spawn_daemon() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("narrate").arg("_record-daemon");

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
        eprintln!("Not recording. Run `attend narrate toggle` or `attend narrate start` to begin.");
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
/// waits for stop/flush sentinels, transcribes, merges, and writes output.
pub fn daemon() -> anyhow::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let config = Config::load(&cwd);
    let engine = config.engine.unwrap_or(Engine::Parakeet);
    let model_path = config.model.unwrap_or_else(|| engine.default_model_path());
    let session_id = resolve_session(None);

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

    // Start editor polling and file diff tracking on background threads.
    // Pass None for cwd so paths stay absolute — filtering is deferred to receive.
    let editor_events = capture::start(None)?;

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

/// Transcribe audio, merge with editor events, and write the pending file as JSON.
fn transcribe_and_write(
    transcriber: &mut dyn super::transcribe::Transcriber,
    recording: audio::Recording,
    editor_snapshots: Vec<Event>,
    file_diffs: Vec<Event>,
    time_base_secs: f64,
    session_id: &Option<String>,
) -> anyhow::Result<()> {
    if recording.duration_secs() < 0.5 {
        tracing::debug!(
            duration_secs = recording.duration_secs(),
            "Recording too short, discarding."
        );
        return Ok(());
    }

    tracing::info!(
        duration_secs = recording.duration_secs(),
        "Transcribing audio..."
    );

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

    // Sort and compress/merge events, then serialize as JSON with absolute paths.
    merge::compress_and_merge(&mut events);

    if events.is_empty() {
        tracing::debug!("No content captured, discarding.");
        return Ok(());
    }

    let json = serde_json::to_string(&events)?;

    let ts = utc_now().replace(':', "-");
    if let Some(sid) = session_id {
        let dir = pending_dir(sid);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{ts}.json"));
        fs::write(&path, &json)?;
        tracing::info!(path = %path.display(), "Narration written");
    } else {
        let path = cache_dir().join("narration.json");
        fs::write(&path, &json)?;
        tracing::info!(path = %path.display(), "Narration written");
    }

    Ok(())
}

