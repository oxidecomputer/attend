//! Recording daemon: captures audio, editor state, and file diffs.
//!
//! The recorder spawns as a detached child process via `_record-daemon`.
//! The parent (toggle/start) exits immediately so the hotkey returns fast.
//! The daemon records until a stop sentinel file appears, then transcribes,
//! merges all streams, and writes the result as a pending narration file.

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use camino::Utf8Path;

use super::audio::{self, AudioChunk};
use super::capture;
use super::merge::{self, Event};
use super::silence::SilenceDetector;
use super::transcribe::{Engine, Word};
use super::{
    cache_dir, flush_sentinel_path, pending_dir, record_lock_path, resolve_session,
    stop_sentinel_path,
};
use crate::config::Config;
use crate::state::SessionId;
use crate::util::utc_now;

/// Target sample rate for transcription engines (Whisper, Parakeet).
const TRANSCRIPTION_SAMPLE_RATE: u32 = 16_000;

/// Number of poll iterations to wait for sentinel acknowledgement.
const SENTINEL_WAIT_ITERATIONS: usize = 100;

/// Interval between sentinel poll checks (ms).
const SENTINEL_POLL_MS: u64 = 50;

/// Main daemon loop poll interval (ms).
const DAEMON_LOOP_POLL_MS: u64 = 100;

/// Number of recent words to feed as transcription context.
const TRANSCRIPTION_CONTEXT_WORDS: usize = 50;

/// Minimum remaining audio duration (seconds) worth transcribing at stop/flush.
const MIN_TRANSCRIPTION_DURATION_SECS: f64 = 0.5;

// ---------------------------------------------------------------------------
// Deferred transcriber (background model loading)
// ---------------------------------------------------------------------------

/// A transcriber that loads its model on a background thread.
///
/// Audio capture and chime playback proceed concurrently with model loading.
/// The first call to [`get()`] blocks until the model is ready.
struct DeferredTranscriber {
    handle: Option<thread::JoinHandle<anyhow::Result<Box<dyn super::transcribe::Transcriber>>>>,
    transcriber: Option<Box<dyn super::transcribe::Transcriber>>,
}

impl DeferredTranscriber {
    /// Spawn model preloading on a background thread.
    fn spawn(engine: Engine, model_path: camino::Utf8PathBuf) -> Self {
        let handle = thread::spawn(move || engine.preload(&model_path));
        Self {
            handle: Some(handle),
            transcriber: None,
        }
    }

    /// Get a mutable reference to the transcriber, blocking if still loading.
    fn get(&mut self) -> anyhow::Result<&mut dyn super::transcribe::Transcriber> {
        if self.transcriber.is_none() {
            let handle = self
                .handle
                .take()
                .expect("transcriber handle already consumed");
            self.transcriber = Some(
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("model preload thread panicked"))??,
            );
        }
        Ok(&mut **self.transcriber.as_mut().unwrap())
    }
}

// ---------------------------------------------------------------------------
// Daemon state
// ---------------------------------------------------------------------------

/// Mutable state for the recording daemon's main loop.
///
/// Bundles audio buffers, transcription state, silence detection, capture
/// handles, and timing into a single struct. The main loop becomes:
///
/// ```ignore
/// loop {
///     state.ingest_chunks()?;
///     if state.check_stop()? { break; }
///     if state.check_flush()? { continue; }
///     thread::sleep(POLL_INTERVAL);
/// }
/// ```
struct DaemonState {
    transcriber: DeferredTranscriber,
    /// Audio capture handle. `Some` during recording, `None` after stop.
    audio_capture: Option<audio::CaptureHandle>,
    /// Editor/diff capture handle. `Some` during recording, `None` after stop.
    editor_capture: Option<capture::CaptureHandle>,
    /// Set by the SIGTERM handler for graceful shutdown.
    terminated: Arc<AtomicBool>,
    silence_detector: Option<SilenceDetector>,
    /// Audio chunks buffered since the last on-the-fly transcription.
    buffered_chunks: Vec<AudioChunk>,
    /// Words transcribed on the fly during this period.
    pre_transcribed: Vec<Word>,
    /// When the current period started (for computing word offsets).
    period_start: Instant,
    /// Wall-clock time when the current period started (for browser event offsets).
    period_start_utc: chrono::DateTime<chrono::Utc>,
    /// Accumulated time base across flushes (seconds).
    time_base_secs: f64,
    /// When the last drain/flush occurred.
    last_drain: Instant,
    sample_rate: u32,
    session_id: Option<SessionId>,
    stop_sentinel: camino::Utf8PathBuf,
    flush_sentinel: camino::Utf8PathBuf,
}

impl DaemonState {
    /// Ingest new audio chunks from the capture stream.
    ///
    /// Feeds each chunk to the silence detector. When a speech segment ends
    /// (silence detected after speech), transcribes the segment on the fly
    /// and frees the audio.
    fn ingest_chunks(&mut self) -> anyhow::Result<()> {
        let capture = self
            .audio_capture
            .as_ref()
            .expect("audio capture already stopped");
        for chunk in capture.take_chunks() {
            if let Some(ref mut detector) = self.silence_detector
                && let Some(silence_start) = detector.feed(&chunk)
            {
                self.buffered_chunks.push(chunk);

                // Partition: chunks whose instant < silence_start are speech.
                let split = self
                    .buffered_chunks
                    .partition_point(|c| c.instant < silence_start);
                let speech: Vec<_> = self.buffered_chunks.drain(..split).collect();
                // Discard trailing silence chunks.
                self.buffered_chunks.clear();
                detector.reset();

                if !speech.is_empty() {
                    self.transcribe_segment(&speech)?;
                }
                continue;
            }
            self.buffered_chunks.push(chunk);
        }
        Ok(())
    }

    /// Check for stop sentinel or SIGTERM. If found, finalize and write narration.
    ///
    /// Returns `true` if the daemon should exit (stop was handled).
    fn check_stop(&mut self) -> anyhow::Result<bool> {
        if !self.stop_sentinel.exists() && !self.terminated.load(Ordering::Relaxed) {
            return Ok(false);
        }

        // Intentionally ignored: chime failure non-fatal.
        let _ = audio::play_chime(false);

        // Grab any final chunks that arrived after the last ingest.
        let capture = self
            .audio_capture
            .take()
            .expect("audio capture already stopped");
        let recording = capture.stop();
        self.buffered_chunks.extend(recording.chunks);
        // Best-effort cleanup.
        let _ = fs::remove_file(&self.stop_sentinel);

        let editor = self
            .editor_capture
            .take()
            .expect("editor capture already stopped");
        let (editor_snapshots, file_diffs, ext_selections) = editor.collect();
        let browser_selections = self.collect_browser_staging();
        self.transcribe_and_write(
            editor_snapshots,
            file_diffs,
            ext_selections,
            browser_selections,
        )?;
        Ok(true)
    }

    /// Check for flush sentinel. If found, write current narration and reset.
    ///
    /// Returns `true` if a flush was handled (caller should `continue`).
    fn check_flush(&mut self) -> anyhow::Result<bool> {
        if !self.flush_sentinel.exists() {
            return Ok(false);
        }

        // Intentionally ignored: chime failure non-fatal.
        let _ = audio::play_flush_chime();

        let capture = self
            .audio_capture
            .as_ref()
            .expect("audio capture already stopped");
        let recording = capture.drain();
        self.buffered_chunks.extend(recording.chunks);
        let elapsed = self.last_drain.elapsed().as_secs_f64();

        let editor = self
            .editor_capture
            .as_ref()
            .expect("editor capture already stopped");
        let (editor_snapshots, file_diffs, ext_selections) = editor.drain();
        let browser_selections = self.collect_browser_staging();
        self.transcribe_and_write(
            editor_snapshots,
            file_diffs,
            ext_selections,
            browser_selections,
        )?;

        self.time_base_secs += elapsed;
        self.last_drain = Instant::now();
        self.period_start = Instant::now();
        self.period_start_utc = chrono::Utc::now();

        if let Some(ref mut detector) = self.silence_detector {
            detector.reset();
        }

        // Acknowledge flush by deleting sentinel (best-effort).
        let _ = fs::remove_file(&self.flush_sentinel);
        Ok(true)
    }

    /// Transcribe a completed speech segment on the fly.
    fn transcribe_segment(&mut self, speech_chunks: &[AudioChunk]) -> anyhow::Result<()> {
        let offset = speech_chunks[0]
            .instant
            .duration_since(self.period_start)
            .as_secs_f64();
        let samples_16k = audio::resample(
            &audio::flatten_chunks(speech_chunks),
            self.sample_rate,
            TRANSCRIPTION_SAMPLE_RATE,
        )?;

        tracing::debug!(
            duration_secs = samples_16k.len() as f64 / TRANSCRIPTION_SAMPLE_RATE as f64,
            offset_secs = offset,
            "Transcribing speech segment on the fly"
        );

        let transcriber = self.transcriber.get()?;
        let words = transcriber.transcribe(&samples_16k)?;
        for w in &words {
            self.pre_transcribed.push(Word {
                start_secs: w.start_secs + offset,
                end_secs: w.end_secs + offset,
                text: w.text.clone(),
            });
        }

        // Feed context for next segment (Whisper uses it, Parakeet ignores).
        let ctx: String = self
            .pre_transcribed
            .iter()
            .rev()
            .take(TRANSCRIPTION_CONTEXT_WORDS)
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        self.transcriber.get()?.set_context(&ctx);

        Ok(())
    }

    /// Collect staged browser selection events for this session.
    ///
    /// File timestamps are converted to offset_secs relative to the current
    /// recording period so browser events interleave with speech and editor events.
    /// Returns a [`BrowserStaging`] whose files are cleaned up only after the
    /// narration is written to disk (crash-safe).
    fn collect_browser_staging(&self) -> super::BrowserStaging {
        self.session_id
            .as_ref()
            .map(|sid| {
                super::collect_browser_staging(sid, self.period_start_utc, self.time_base_secs)
            })
            .unwrap_or_default()
    }

    /// Transcribe remaining audio, combine with pre-transcribed words, merge
    /// with editor events, and write the pending narration file.
    fn transcribe_and_write(
        &mut self,
        editor_snapshots: Vec<Event>,
        file_diffs: Vec<Event>,
        ext_selections: Vec<Event>,
        browser_staging: super::BrowserStaging,
    ) -> anyhow::Result<()> {
        let remaining_chunks = std::mem::take(&mut self.buffered_chunks);
        let mut all_words = std::mem::take(&mut self.pre_transcribed);

        // Transcribe any remaining buffered audio (the in-progress segment).
        if !remaining_chunks.is_empty() {
            let remaining_samples = audio::flatten_chunks(&remaining_chunks);
            let remaining_duration = remaining_samples.len() as f64 / self.sample_rate as f64;

            if remaining_duration >= MIN_TRANSCRIPTION_DURATION_SECS {
                let offset = remaining_chunks[0]
                    .instant
                    .duration_since(self.period_start)
                    .as_secs_f64();

                tracing::info!(
                    duration_secs = remaining_duration,
                    "Transcribing remaining audio..."
                );

                let samples_16k = audio::resample(
                    &remaining_samples,
                    self.sample_rate,
                    TRANSCRIPTION_SAMPLE_RATE,
                )?;
                let transcriber = self.transcriber.get()?;
                let words = transcriber.transcribe(&samples_16k)?;
                for w in &words {
                    all_words.push(Word {
                        start_secs: w.start_secs + offset,
                        end_secs: w.end_secs + offset,
                        text: w.text.clone(),
                    });
                }
            } else {
                tracing::debug!(
                    duration_secs = remaining_duration,
                    "Remaining audio too short, skipping."
                );
            }
        }

        if all_words.is_empty()
            && editor_snapshots.is_empty()
            && file_diffs.is_empty()
            && ext_selections.is_empty()
            && browser_staging.events.is_empty()
        {
            tracing::debug!("No content captured, discarding.");
            return Ok(());
        }

        let mut events: Vec<Event> = Vec::new();

        for word in &all_words {
            events.push(Event::Words {
                offset_secs: word.start_secs + self.time_base_secs,
                text: word.text.clone(),
            });
        }

        events.extend(editor_snapshots);
        events.extend(file_diffs);
        events.extend(ext_selections);
        let (browser_events, browser_cleanup) = browser_staging.take();
        events.extend(browser_events);

        // Sort and compress/merge events, then serialize as JSON.
        merge::compress_and_merge(&mut events);

        if events.is_empty() {
            tracing::debug!("No content captured after merge, discarding.");
            return Ok(());
        }

        let json = serde_json::to_string(&events)?;

        let ts = utc_now().replace(':', "-");
        if let Some(ref sid) = self.session_id {
            let dir = pending_dir(sid);
            fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{ts}.json"));
            crate::util::atomic_write_str(&path, &json)?;
            tracing::info!(path = %path, "Narration written");
        } else {
            let path = cache_dir().join("narration.json");
            crate::util::atomic_write_str(&path, &json)?;
            tracing::info!(path = %path, "Narration written");
        }

        // Only remove browser staging files after narration is safely on disk.
        browser_cleanup.cleanup();

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public API: toggle, start, stop, daemon
// ---------------------------------------------------------------------------

/// Toggle recording: start if not recording, stop if recording.
pub fn toggle() -> anyhow::Result<()> {
    let lock = record_lock_path();
    if lock.exists() {
        // Check for stale lock (daemon was killed without cleanup).
        if is_lock_stale(&lock) {
            tracing::warn!("Stale record lock detected, cleaning up.");
            // Best-effort cleanup: files may already be gone.
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
pub(crate) fn is_lock_stale(lock_path: &Utf8Path) -> bool {
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
            // Best-effort cleanup: files may already be gone.
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
    for _ in 0..SENTINEL_WAIT_ITERATIONS {
        if !sentinel.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(SENTINEL_POLL_MS));
    }

    eprintln!("Flush signal sent; daemon may still be transcribing.");
    Ok(())
}

/// Spawn a detached recording daemon process.
fn spawn_daemon() -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;

    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("narrate").arg("_record-daemon");

    // Put the child in its own process group immediately (before exec).
    // This closes the race window where Zed's task runner kills the parent's
    // process group before the daemon has a chance to call setsid().
    // The daemon still calls setsid() at startup for full session isolation.
    cmd.process_group(0);

    // Detach stdio so the daemon doesn't hold the parent's descriptors.
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    cmd.spawn()?;

    // No grace period needed: the daemon acquires the record lock at startup.
    // If the user double-toggles, the second spawn will find the lock held
    // and the toggle logic handles it (stop or stale-lock cleanup).

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
    for _ in 0..SENTINEL_WAIT_ITERATIONS {
        if !record_lock_path().exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(SENTINEL_POLL_MS));
    }

    eprintln!("Stop signal sent; daemon may still be transcribing.");
    Ok(())
}

/// The actual recording daemon entry point.
///
/// Acquires the record lock, captures audio + editor state + file diffs,
/// waits for stop/flush sentinels, transcribes, merges, and writes output.
///
/// When silence-based segmentation is enabled (the default), completed speech
/// segments are transcribed on the fly and their audio is freed. At stop/flush,
/// only the current in-progress segment needs transcription.
pub fn daemon() -> anyhow::Result<()> {
    // Create a new session so the daemon survives if the parent's process
    // group is killed (e.g. Zed's task runner cleaning up after toggle exits).
    // Intentionally ignored: may fail if already session leader (e.g. run manually).
    let _ = nix::unistd::setsid();

    let cwd = camino::Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
        .unwrap_or_else(|_| camino::Utf8PathBuf::from("."));
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

    // Best-effort PID write for stale lock detection. This is not atomic
    // with the lock creation above: if the process is SIGKILL'd between
    // Lockfile::create and this write, the lock file will exist without a
    // PID, making is_lock_stale() return false (permanently stuck lock).
    // This is acceptable because:
    // - SIGTERM is caught by our signal handler and triggers clean shutdown
    //   (Lockfile::Drop removes the file).
    // - Only SIGKILL can cause the stuck state, and that's unrecoverable
    //   by design.
    let _ = fs::write(record_lock_path(), std::process::id().to_string());

    // Best-effort cleanup: sentinels may not exist.
    let _ = fs::remove_file(stop_sentinel_path());
    let _ = fs::remove_file(flush_sentinel_path());

    // Start audio capture immediately — audio accumulates in the background
    // while the model loads and the chime plays.
    let capture = audio::start_capture()?;
    let sample_rate = capture.sample_rate();

    // Start editor polling, file diff tracking, and external selection capture
    // on background threads. Pass None for cwd so paths stay absolute — filtering
    // is deferred to receive.
    let editor_events = capture::start(None, config.ext_ignore_apps.clone())?;

    // Spawn model preload on a background thread. The first call to
    // transcriber.get() blocks until the model is ready. This lets audio
    // accumulate and the chime play concurrently with model loading.
    let transcriber = DeferredTranscriber::spawn(engine, model_path);

    // Register SIGTERM handler so `kill <pid>` triggers a clean shutdown
    // (transcribe remaining audio and release the lock) instead of an abrupt exit.
    let terminated = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&terminated))?;

    // Intentionally ignored: chime failure should not abort recording.
    let _ = audio::play_chime(true);

    // Silence-based segmentation (0 disables).
    let silence_secs = config.silence_duration.unwrap_or(5.0);
    let silence_detector = if silence_secs > 0.0 {
        Some(SilenceDetector::new(
            sample_rate,
            Duration::from_secs_f64(silence_secs),
        ))
    } else {
        None
    };

    let now = Instant::now();
    let mut state = DaemonState {
        transcriber,
        audio_capture: Some(capture),
        editor_capture: Some(editor_events),
        terminated,
        silence_detector,
        buffered_chunks: Vec::new(),
        pre_transcribed: Vec::new(),
        period_start: now,
        period_start_utc: chrono::Utc::now(),
        time_base_secs: 0.0,
        last_drain: now,
        sample_rate,
        session_id,
        stop_sentinel: stop_sentinel_path(),
        flush_sentinel: flush_sentinel_path(),
    };

    loop {
        state.ingest_chunks()?;
        if state.check_stop()? {
            break;
        }
        if state.check_flush()? {
            continue;
        }
        thread::sleep(Duration::from_millis(DAEMON_LOOP_POLL_MS));
    }

    Ok(())
}
