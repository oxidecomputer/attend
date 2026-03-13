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
use std::time::Duration;

use chrono::{DateTime, Utc};

use camino::Utf8Path;

use super::audio::{self, AudioChunk};
use super::capture;
use super::chime::Chime;
use super::merge::{self, Event};
use super::silence::SilenceDetector;
use super::transcribe::{Engine, Word};
use super::{
    flush_sentinel_path, pause_sentinel_path, pending_dir, record_lock_path, resolve_session,
    stop_sentinel_path, yank_sentinel_path, yanked_dir,
};
use crate::clock::{Clock, SyncClock};
use crate::config::Config;
use crate::state::SessionId;
use crate::util::format_utc_nanos;

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

    /// Create with a pre-loaded transcriber (test mode: no model loading).
    fn preloaded(transcriber: Box<dyn super::transcribe::Transcriber>) -> Self {
        Self {
            handle: None,
            transcriber: Some(transcriber),
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
    clock: Arc<dyn SyncClock>,
    transcriber: DeferredTranscriber,
    /// Audio capture handle. `Some` during recording, `None` after finalize.
    audio_capture: Option<Box<dyn audio::AudioSource>>,
    /// Editor/diff capture handle. `Some` during recording, `None` after finalize.
    editor_capture: Option<capture::CaptureHandle>,
    /// Set by the SIGTERM handler for graceful shutdown.
    terminated: Arc<AtomicBool>,
    silence_detector: Option<SilenceDetector>,
    /// Audio chunks buffered since the last on-the-fly transcription.
    buffered_chunks: Vec<AudioChunk>,
    /// Words transcribed on the fly during this period.
    pre_transcribed: Vec<Word>,
    /// When the current period started (UTC).
    /// Word timestamps are computed as `period_start + segment_offset + word.start_secs`.
    period_start: DateTime<Utc>,
    /// Accumulated offset across flushes: added to audio segment offsets
    /// so word timestamps account for previous periods.
    time_base_secs: f64,
    /// When the last drain/flush occurred (UTC).
    last_drain: DateTime<Utc>,
    sample_rate: u32,
    session_id: Option<SessionId>,
    stop_sentinel: camino::Utf8PathBuf,
    flush_sentinel: camino::Utf8PathBuf,
    pause_sentinel: camino::Utf8PathBuf,
    yank_sentinel: camino::Utf8PathBuf,
    /// Whether the daemon is currently paused (user-initiated or idle).
    paused: bool,
    /// When the current pause started (for time_base_secs adjustment on resume).
    pause_started_at: Option<DateTime<Utc>>,
    /// Whether the daemon is idle (entered via stop, model stays loaded).
    /// Distinct from user-initiated pause: resume from idle re-resolves
    /// the session and resets timing for a fresh recording period.
    idle: bool,
    /// When the daemon entered idle state (for timeout tracking).
    idle_since: Option<DateTime<Utc>>,
    /// How long to stay idle before auto-exiting. `None` = forever.
    idle_timeout: Option<Duration>,
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

                // Partition: chunks whose timestamp < silence_start are speech.
                let split = self
                    .buffered_chunks
                    .partition_point(|c| c.timestamp < silence_start);
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

    /// Check for SIGTERM. If received, finalize and exit.
    ///
    /// When idle (already flushed), exits immediately. When recording,
    /// finalizes all streams before exiting.
    ///
    /// Returns `true` if the daemon should exit.
    fn check_terminated(&mut self) -> anyhow::Result<bool> {
        if !self.terminated.load(Ordering::Relaxed) {
            return Ok(false);
        }

        // If idle, content was already flushed by check_stop. Just exit.
        if self.idle {
            tracing::info!("SIGTERM while idle, exiting.");
            return Ok(true);
        }

        let to_clipboard = self.session_id.is_none();
        self.finalize_and_write(
            if to_clipboard {
                yanked_dir(self.session_id.as_ref())
            } else {
                pending_dir(self.session_id.as_ref())
            },
            if to_clipboard {
                Chime::Yank
            } else {
                Chime::Stop
            },
            &self.stop_sentinel.clone(),
        )
    }

    /// Check for stop sentinel. Flush content and enter idle state.
    ///
    /// Unlike SIGTERM/yank, stop keeps the daemon alive with the model
    /// loaded. The daemon enters idle (paused) state and waits for a
    /// resume signal or idle timeout.
    fn check_stop(&mut self) -> anyhow::Result<()> {
        if !self.stop_sentinel.exists() {
            return Ok(());
        }

        let to_clipboard = self.session_id.is_none();
        let chime = if to_clipboard {
            Chime::Yank
        } else {
            Chime::Stop
        };
        // Intentionally ignored: chime failure non-fatal.
        let _ = chime.play();

        // Drain (not stop) audio and editor — handles stay alive for resume.
        let capture = self
            .audio_capture
            .as_ref()
            .expect("audio capture already stopped");
        let recording = capture.drain();
        self.buffered_chunks.extend(recording.chunks);

        let editor = self
            .editor_capture
            .as_ref()
            .expect("editor capture already stopped");
        let (editor_snapshots, file_diffs, ext_selections, clipboard_selections) = editor.drain();
        let browser_staging = self.collect_browser_staging();
        let shell_staging = self.collect_shell_staging();

        let dest = if to_clipboard {
            yanked_dir(self.session_id.as_ref())
        } else {
            pending_dir(self.session_id.as_ref())
        };
        let had_content = self.transcribe_and_write_to(
            dest,
            editor_snapshots,
            file_diffs,
            ext_selections,
            clipboard_selections,
            browser_staging,
            shell_staging,
        )?;
        if !had_content {
            // Intentionally ignored: chime failure non-fatal.
            let _ = Chime::Empty.play();
        }

        // Acknowledge stop by deleting sentinel.
        let _ = fs::remove_file(&self.stop_sentinel);

        // Reset timing and context for next recording period.
        let now = self.clock.now();
        self.time_base_secs = 0.0;
        self.last_drain = now;
        self.period_start = now;
        self.pre_transcribed.clear();
        self.buffered_chunks.clear();

        if let Some(ref mut detector) = self.silence_detector {
            detector.reset();
        }

        // Suspend all capture.
        if let Some(ref audio) = self.audio_capture {
            // Intentionally ignored: pause failure non-fatal.
            let _ = audio.pause();
        }
        if let Some(ref mut editor) = self.editor_capture {
            editor.pause();
        }

        // Write pause sentinel so CLI knows we're idle.
        if let Some(parent) = self.pause_sentinel.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&self.pause_sentinel, "");

        self.paused = true;
        self.idle = true;
        self.idle_since = Some(self.clock.now());
        self.pause_started_at = Some(self.clock.now());

        tracing::info!("Stop handled: flushed content, entering idle state.");

        Ok(())
    }

    /// Check for yank sentinel. If found, finalize and write narration to
    /// the `yanked/` directory (instead of `pending/`) so the yank CLI can
    /// read it without racing with hook delivery.
    ///
    /// Returns `true` if the daemon should exit (yank was handled).
    fn check_yank(&mut self) -> anyhow::Result<bool> {
        if !self.yank_sentinel.exists() {
            return Ok(false);
        }

        self.finalize_and_write(
            yanked_dir(self.session_id.as_ref()),
            Chime::Yank,
            &self.yank_sentinel.clone(),
        )
    }

    /// Shared finalization: play chime, collect all streams, transcribe,
    /// and write to the given destination directory.
    fn finalize_and_write(
        &mut self,
        dest_dir: camino::Utf8PathBuf,
        chime: Chime,
        sentinel: &camino::Utf8Path,
    ) -> anyhow::Result<bool> {
        // Intentionally ignored: chime failure non-fatal.
        let _ = chime.play();

        // Grab any final chunks that arrived after the last ingest.
        let recording = self
            .audio_capture
            .as_mut()
            .expect("audio capture already stopped")
            .stop();
        self.audio_capture = None;
        self.buffered_chunks.extend(recording.chunks);
        // Best-effort cleanup.
        let _ = fs::remove_file(sentinel);

        let editor = self
            .editor_capture
            .take()
            .expect("editor capture already stopped");
        let (editor_snapshots, file_diffs, ext_selections, clipboard_selections) = editor.collect();
        let browser_staging = self.collect_browser_staging();
        let shell_staging = self.collect_shell_staging();
        let had_content = self.transcribe_and_write_to(
            dest_dir,
            editor_snapshots,
            file_diffs,
            ext_selections,
            clipboard_selections,
            browser_staging,
            shell_staging,
        )?;
        if !had_content {
            // Intentionally ignored: chime failure non-fatal.
            let _ = Chime::Empty.play();
        }
        Ok(true)
    }

    /// Check for flush sentinel. If found, write current narration and reset.
    ///
    /// When no agent session is active, writes to `yanked/` instead of
    /// `pending/` (same as stop without a session).
    ///
    /// Returns `true` if a flush was handled (caller should `continue`).
    fn check_flush(&mut self) -> anyhow::Result<bool> {
        if !self.flush_sentinel.exists() {
            return Ok(false);
        }

        let to_clipboard = self.session_id.is_none();

        // Yank chime when going to clipboard, flush chime when going to agent.
        let _ = if to_clipboard {
            Chime::Yank.play()
        } else {
            Chime::Flush.play()
        };

        let capture = self
            .audio_capture
            .as_ref()
            .expect("audio capture already stopped");
        let recording = capture.drain();
        self.buffered_chunks.extend(recording.chunks);
        let now = self.clock.now();
        let elapsed = (now - self.last_drain)
            .to_std()
            .unwrap_or_default()
            .as_secs_f64();

        let editor = self
            .editor_capture
            .as_ref()
            .expect("editor capture already stopped");
        let (editor_snapshots, file_diffs, ext_selections, clipboard_selections) = editor.drain();
        let browser_staging = self.collect_browser_staging();
        let shell_staging = self.collect_shell_staging();

        let dest = if to_clipboard {
            yanked_dir(self.session_id.as_ref())
        } else {
            pending_dir(self.session_id.as_ref())
        };
        let had_content = self.transcribe_and_write_to(
            dest,
            editor_snapshots,
            file_diffs,
            ext_selections,
            clipboard_selections,
            browser_staging,
            shell_staging,
        )?;
        if !had_content {
            // Intentionally ignored: chime failure non-fatal.
            let _ = Chime::Empty.play();
        }

        self.time_base_secs += elapsed;
        self.last_drain = now;
        self.period_start = now;

        if let Some(ref mut detector) = self.silence_detector {
            detector.reset();
        }

        // Acknowledge flush by deleting sentinel (best-effort).
        let _ = fs::remove_file(&self.flush_sentinel);
        Ok(true)
    }

    /// Check for pause/resume transitions.
    ///
    /// On pause (sentinel appears): suspend capture without flushing.
    /// Content stays buffered — audio in `buffered_chunks`, editor events
    /// in capture thread buffers, staging files on disk. Everything is
    /// written together at the next stop or flush.
    ///
    /// On resume from user-pause (sentinel disappears while paused, not idle):
    /// resume capture. Timing is not reset — wall-clock timestamps span the gap.
    ///
    /// On resume from idle (sentinel disappears while idle): start a fresh
    /// recording session. Re-resolves the session ID (may have changed while
    /// idle), resets timing, and plays the start chime.
    fn check_pause(&mut self) -> anyhow::Result<()> {
        let sentinel_exists = self.pause_sentinel.exists();

        if sentinel_exists && !self.paused {
            // Transition: recording -> paused.
            tracing::info!("Pause detected, suspending capture.");

            // Intentionally ignored: chime failure non-fatal.
            let _ = Chime::Pause.play();

            // Drain remaining audio from the device buffer so it isn't lost
            // during the pause. Chunks stay in self.buffered_chunks — they
            // are NOT transcribed or written to pending.
            let capture = self
                .audio_capture
                .as_ref()
                .expect("audio capture already stopped");
            let recording = capture.drain();
            self.buffered_chunks.extend(recording.chunks);

            // Suspend all capture.
            if let Some(ref audio) = self.audio_capture {
                // Intentionally ignored: pause failure non-fatal.
                let _ = audio.pause();
            }
            if let Some(ref mut editor) = self.editor_capture {
                editor.pause();
            }

            if let Some(ref mut detector) = self.silence_detector {
                detector.reset();
            }

            self.paused = true;
            self.pause_started_at = Some(self.clock.now());
        } else if !sentinel_exists && self.paused {
            if self.idle {
                // Transition: idle -> recording (new session).
                tracing::info!("Resume from idle, starting new recording session.");

                // Intentionally ignored: chime failure non-fatal.
                let _ = Chime::Start.play();

                // Re-resolve session: it may have changed while idle.
                self.session_id = resolve_session(None);

                // Resume all capture.
                if let Some(ref audio) = self.audio_capture {
                    // Intentionally ignored: resume failure non-fatal.
                    let _ = audio.resume();
                }
                if let Some(ref mut editor) = self.editor_capture {
                    editor.resume();
                }

                // Fresh timing for the new recording period.
                let now = self.clock.now();
                self.period_start = now;
                self.time_base_secs = 0.0;
                self.last_drain = now;

                self.pre_transcribed.clear();

                if let Some(ref mut detector) = self.silence_detector {
                    detector.reset();
                }

                self.paused = false;
                self.idle = false;
                self.idle_since = None;
                self.pause_started_at = None;
            } else {
                // Transition: paused -> recording (resume existing session).
                tracing::info!("Resume detected, resuming capture.");

                // Intentionally ignored: chime failure non-fatal.
                let _ = Chime::Resume.play();

                // Resume all capture.
                if let Some(ref audio) = self.audio_capture {
                    // Intentionally ignored: resume failure non-fatal.
                    let _ = audio.resume();
                }
                if let Some(ref mut editor) = self.editor_capture {
                    editor.resume();
                }

                self.pause_started_at = None;

                // Reset the silence detector: there is an audio discontinuity
                // at the pause/resume boundary, so prior detector state is stale.
                if let Some(ref mut detector) = self.silence_detector {
                    detector.reset();
                }

                self.paused = false;
            }
        }

        Ok(())
    }

    /// Check whether the idle timeout has elapsed.
    ///
    /// Returns `true` if the daemon should exit (has been idle too long).
    fn check_idle_timeout(&self) -> bool {
        if let Some(timeout) = self.idle_timeout
            && let Some(since) = self.idle_since
        {
            let elapsed = (self.clock.now() - since).to_std().unwrap_or_default();
            return elapsed > timeout;
        }
        false
    }

    /// Transcribe a completed speech segment on the fly.
    fn transcribe_segment(&mut self, speech_chunks: &[AudioChunk]) -> anyhow::Result<()> {
        let offset = (speech_chunks[0].timestamp - self.period_start)
            .to_std()
            .unwrap_or_default()
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
    /// Returns a [`StagingResult`] whose files are cleaned up only after the
    /// narration is written to disk (crash-safe).
    fn collect_browser_staging(&self) -> super::StagingResult {
        super::collect_browser_staging(
            self.session_id.as_ref(),
            self.period_start,
            self.clock.now(),
        )
    }

    /// Collect staged shell command events for this session.
    fn collect_shell_staging(&self) -> super::StagingResult {
        super::collect_shell_staging(
            self.session_id.as_ref(),
            self.period_start,
            self.clock.now(),
        )
    }

    /// Transcribe remaining audio, combine with pre-transcribed words, merge
    /// with editor events, and write to the specified output directory.
    ///
    /// Returns `true` if content was produced, `false` if nothing was captured.
    #[allow(clippy::too_many_arguments)]
    fn transcribe_and_write_to(
        &mut self,
        dest_dir: camino::Utf8PathBuf,
        editor_snapshots: Vec<Event>,
        file_diffs: Vec<Event>,
        ext_selections: Vec<Event>,
        clipboard_selections: Vec<Event>,
        browser_staging: super::StagingResult,
        shell_staging: super::StagingResult,
    ) -> anyhow::Result<bool> {
        let remaining_chunks = std::mem::take(&mut self.buffered_chunks);
        let mut all_words = std::mem::take(&mut self.pre_transcribed);

        // Transcribe any remaining buffered audio (the in-progress segment).
        // Always transcribe regardless of duration: the silence detector
        // already identified speech boundaries, and short utterances between
        // pauses (e.g., counting while clicking) are real speech.
        if !remaining_chunks.is_empty() {
            let remaining_samples = audio::flatten_chunks(&remaining_chunks);
            let offset = (remaining_chunks[0].timestamp - self.period_start)
                .to_std()
                .unwrap_or_default()
                .as_secs_f64();

            tracing::info!(
                duration_secs = remaining_samples.len() as f64 / self.sample_rate as f64,
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
        }

        if all_words.is_empty()
            && editor_snapshots.is_empty()
            && file_diffs.is_empty()
            && ext_selections.is_empty()
            && clipboard_selections.is_empty()
            && browser_staging.events.is_empty()
            && shell_staging.events.is_empty()
        {
            tracing::debug!("No content captured, discarding.");
            return Ok(false);
        }

        let mut events: Vec<Event> = Vec::new();

        for word in &all_words {
            let secs = word.start_secs + self.time_base_secs;
            let timestamp =
                self.period_start + chrono::Duration::milliseconds((secs * 1000.0) as i64);
            events.push(Event::Words {
                timestamp,
                text: word.text.clone(),
            });
        }

        events.extend(editor_snapshots);
        events.extend(file_diffs);
        events.extend(ext_selections);
        events.extend(clipboard_selections);
        let (browser_events, browser_cleanup) = browser_staging.take();
        events.extend(browser_events);
        let (shell_events, shell_cleanup) = shell_staging.take();
        events.extend(shell_events);

        // Sort and compress/merge events, then serialize as JSON.
        merge::compress_and_merge(&mut events);

        if events.is_empty() {
            tracing::debug!("No content captured after merge, discarding.");
            return Ok(false);
        }

        let json = serde_json::to_string(&events)?;

        let ts = format_utc_nanos(self.clock.now()).replace(':', "-");
        fs::create_dir_all(&dest_dir)?;
        let path = dest_dir.join(format!("{ts}.json"));
        crate::util::atomic_write_str(&path, &json)?;
        tracing::info!(path = %path, "Narration written");

        // Only remove staging files after narration is safely on disk.
        browser_cleanup.cleanup();
        shell_cleanup.cleanup();

        // Clipboard staging images are NOT cleaned up here. Unlike browser/shell
        // staging (which contains event data merged into the narration JSON),
        // clipboard images are referenced by absolute path in the narration
        // output. The agent needs to read them when it encounters
        // `![clipboard](path)` tags. They are cleaned up by archive retention
        // (clean.rs) alongside old narration files.

        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Public API: toggle, start, stop, daemon
// ---------------------------------------------------------------------------

/// Toggle recording: start if idle/stopped, stop if recording.
///
/// With persistent daemon:
/// - **Lock + no pause sentinel** → recording → send stop (daemon enters idle).
/// - **Lock + pause sentinel** → idle → delete pause sentinel (daemon resumes).
/// - **No lock** → spawn new daemon.
pub fn toggle(clock: &dyn SyncClock) -> anyhow::Result<()> {
    let lock = record_lock_path();
    if lock.exists() {
        if is_lock_stale(&lock) {
            tracing::warn!("Stale record lock detected, cleaning up.");
            let _ = fs::remove_file(&lock);
            let _ = fs::remove_file(stop_sentinel_path());
            let _ = fs::remove_file(pause_sentinel_path());
            spawn_daemon()
        } else if pause_sentinel_path().exists() {
            // Daemon is idle (or user-paused). Resume by removing sentinel.
            resume()
        } else {
            // Daemon is recording. Stop it (enters idle).
            stop(clock)
        }
    } else {
        spawn_daemon()
    }
}

/// Resume an idle or paused daemon by removing the pause sentinel.
///
/// The daemon detects the sentinel removal and resumes capture.
fn resume() -> anyhow::Result<()> {
    let sentinel = pause_sentinel_path();
    if sentinel.exists() {
        fs::remove_file(&sentinel)?;
    }
    Ok(())
}

/// Check whether a lock file is stale (the owning process is no longer alive).
///
/// Supports both the new `PID:TIMESTAMP` format (which guards against PID
/// reuse via `process_alive_since`) and the legacy `PID`-only format (which
/// falls back to plain `process_alive`).
pub(crate) fn is_lock_stale(lock_path: &Utf8Path) -> bool {
    let Ok(content) = fs::read_to_string(lock_path) else {
        return false;
    };
    if super::parse_lock_content(content.trim()).is_none() {
        // Unparseable content: can't determine, assume not stale.
        return false;
    }
    !super::lock_owner_alive(&content)
}

/// Start recording, or flush current narration and keep recording.
///
/// With persistent daemon:
/// - **No lock** → spawn new daemon.
/// - **Lock + pause sentinel (idle)** → resume by deleting pause sentinel.
/// - **Lock + recording** → flush (submit current narration, keep recording).
pub fn start(clock: &dyn SyncClock) -> anyhow::Result<()> {
    let lock = record_lock_path();

    if lock.exists() {
        if is_lock_stale(&lock) {
            tracing::warn!("Stale record lock detected, cleaning up.");
            let _ = fs::remove_file(&lock);
            let _ = fs::remove_file(stop_sentinel_path());
            let _ = fs::remove_file(flush_sentinel_path());
            let _ = fs::remove_file(pause_sentinel_path());
            // Fall through to spawn below.
        } else if pause_sentinel_path().exists() {
            // Daemon is idle. Resume it.
            return resume();
        } else {
            return flush(clock);
        }
    }

    spawn_daemon()
}

/// Signal the running daemon to flush (submit current narration, keep recording).
///
/// When no agent session is active, the daemon writes to `yanked/` and
/// this function copies the content to the clipboard.
fn flush(clock: &dyn SyncClock) -> anyhow::Result<()> {
    let session_id = resolve_session(None);
    let to_clipboard = session_id.is_none();

    let sentinel = flush_sentinel_path();
    if let Some(parent) = sentinel.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&sentinel, "")?;

    // Wait for the daemon to delete the sentinel (acknowledging the flush).
    for _ in 0..SENTINEL_WAIT_ITERATIONS {
        if !sentinel.exists() {
            break;
        }
        clock.sleep(Duration::from_millis(SENTINEL_POLL_MS));
    }

    if sentinel.exists() {
        eprintln!("Flush signal sent; daemon may still be transcribing.");
    }

    if to_clipboard {
        copy_yanked_to_clipboard(session_id.as_ref())?;
    }

    Ok(())
}

/// Spawn a detached recording daemon process.
///
/// On macOS, uses `responsibility_spawnattrs_setdisclaim` so that TCC
/// permissions (microphone, accessibility) accrue to the `attend` binary
/// rather than whichever app (Zed, iTerm2, Terminal) spawned the CLI.
fn spawn_daemon() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let mut extra_env: Vec<(&str, String)> = Vec::new();
    let mut stderr_file = None;

    if crate::test_mode::is_active() {
        extra_env.push(("ATTEND_TEST_MODE", "1".to_string()));
        if let Ok(val) = std::env::var("ATTEND_CACHE_DIR") {
            let log_path = std::path::PathBuf::from(&val).join("daemon-stderr.log");
            stderr_file = std::fs::File::create(&log_path).ok();
            extra_env.push(("ATTEND_CACHE_DIR", val));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let extra_env_refs: Vec<(&str, &str)> =
            extra_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let result = macos_disclaim::spawn(macos_disclaim::DisclaimedSpawn {
            exe: exe.as_path(),
            argv: &["attend", "narrate", "_record-daemon"],
            extra_env: &extra_env_refs,
            stderr_file,
        })?;
        if !result.disclaimed {
            tracing::warn!(
                "responsibility_spawnattrs_setdisclaim unavailable: \
                 TCC permissions will accrue to the parent process"
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        use std::os::unix::process::CommandExt;

        let mut cmd = std::process::Command::new(exe);
        cmd.arg("narrate").arg("_record-daemon");
        cmd.process_group(0);
        for (k, v) in &extra_env {
            cmd.env(k, v);
        }
        if let Some(log_file) = stderr_file {
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::from(log_file));
        } else {
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }
        cmd.spawn()?;
    }

    Ok(())
}

/// Signal the recorder to stop by creating the stop sentinel.
///
/// The daemon flushes content and enters idle state (persistent daemon).
/// This function waits for the daemon to acknowledge by deleting the
/// stop sentinel.
///
/// When no agent session is active, the daemon writes to `yanked/` and
/// this function copies the content to the clipboard.
///
/// If not recording (no lock or already idle), this is a no-op.
pub fn stop(clock: &dyn SyncClock) -> anyhow::Result<()> {
    if !record_lock_path().exists() {
        eprintln!("Not recording. Run `attend narrate toggle` or `attend narrate start` to begin.");
        return Ok(());
    }

    // Already idle — nothing to stop.
    if pause_sentinel_path().exists() {
        return Ok(());
    }

    let session_id = resolve_session(None);
    let to_clipboard = session_id.is_none();

    let sentinel = stop_sentinel_path();
    if let Some(parent) = sentinel.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&sentinel, "")?;

    // Wait for the daemon to complete the stop: the stop sentinel is
    // removed AND the pause sentinel appears (daemon has entered idle).
    // Waiting for both conditions eliminates the TOCTOU race where the
    // daemon has deleted the stop sentinel but hasn't finished its state
    // transition yet.
    let pause = pause_sentinel_path();
    for _ in 0..SENTINEL_WAIT_ITERATIONS {
        if !sentinel.exists() && pause.exists() {
            break;
        }
        clock.sleep(Duration::from_millis(SENTINEL_POLL_MS));
    }

    if sentinel.exists() {
        eprintln!("Stop signal sent; daemon may still be transcribing.");
    }

    if to_clipboard {
        copy_yanked_to_clipboard(session_id.as_ref())?;
    }

    Ok(())
}

/// Toggle pause state by writing or removing the pause sentinel.
///
/// If the daemon is recording, writes the sentinel to pause or removes it
/// to resume. If not recording, prints a message and exits.
pub fn pause() -> anyhow::Result<()> {
    if !record_lock_path().exists() {
        eprintln!("Not recording. Run `attend narrate toggle` or `attend narrate start` to begin.");
        return Ok(());
    }

    let sentinel = pause_sentinel_path();
    if sentinel.exists() {
        // Resume: remove the sentinel.
        fs::remove_file(&sentinel)?;
    } else {
        // Pause: create the sentinel.
        if let Some(parent) = sentinel.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&sentinel, "")?;
    }

    Ok(())
}

/// Yank: stop recording and copy rendered narration to the system clipboard.
///
/// Writes the yank sentinel (not stop), waits for the daemon to exit, reads
/// the yanked output, renders to markdown, and copies to clipboard. If no
/// content was captured, prints a message and leaves the clipboard unchanged.
pub fn yank(clock: &dyn SyncClock) -> anyhow::Result<()> {
    if !record_lock_path().exists() {
        eprintln!("Not recording. Run `attend narrate toggle` or `attend narrate start` to begin.");
        return Ok(());
    }

    let sentinel = yank_sentinel_path();
    if let Some(parent) = sentinel.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&sentinel, "")?;

    // Wait for daemon to exit (same pattern as stop).
    for _ in 0..SENTINEL_WAIT_ITERATIONS {
        if !record_lock_path().exists() {
            break;
        }
        clock.sleep(Duration::from_millis(SENTINEL_POLL_MS));
    }

    if record_lock_path().exists() {
        eprintln!("Stop signal sent; daemon may still be transcribing.");
    }

    copy_yanked_to_clipboard(resolve_session(None).as_ref())
}

/// Read yanked narration files, copy to clipboard, archive, and clean up.
///
/// Shared by `yank()` and `stop()` (when no agent is listening).
fn copy_yanked_to_clipboard(session_id: Option<&crate::state::SessionId>) -> anyhow::Result<()> {
    let cwd = camino::Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
        .unwrap_or_else(|_| camino::Utf8PathBuf::from("."));
    let config = crate::config::Config::load(&cwd);

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if let Some(sid) = session_id {
        files.extend(collect_json_files(&super::yanked_dir(Some(sid))));
    }
    files.extend(collect_json_files(&super::yanked_dir(None)));
    files.sort();

    // Filter by cwd when there's a session (the narration is for a specific
    // project). Without a session, include all content unfiltered — the user
    // isn't targeting a particular project and can paste anywhere.
    let cwd_filter = session_id.map(|_| cwd.as_path());
    if let Some(content) = super::receive::read_pending(
        &files,
        cwd_filter,
        &config.include_dirs,
        super::render::RenderMode::Yank,
    ) {
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|e| anyhow::anyhow!("cannot access clipboard: {e}"))?;
        clipboard
            .set_text(&content)
            .map_err(|e| anyhow::anyhow!("cannot write to clipboard: {e}"))?;

        let lines = content.lines().count();
        let chars = content.len();
        eprintln!("Copied {lines} lines ({chars} chars) to clipboard.");
    } else {
        eprintln!("No narration content.");
    }

    // Archive yanked files (same retention/cleanup as pending narrations).
    let archive = super::archive_dir(session_id);
    let _ = fs::create_dir_all(&archive);
    for path in &files {
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            let dest = archive.join(filename);
            let _ = fs::rename(path, dest.as_std_path());
        }
    }
    // Best-effort: only succeeds if empty.
    if let Some(sid) = session_id {
        let _ = fs::remove_dir(super::yanked_dir(Some(sid)));
    }
    let _ = fs::remove_dir(super::yanked_dir(None));

    // Prune old archives.
    super::receive::auto_prune(&config);

    Ok(())
}

/// Collect `.json` files from a directory.
fn collect_json_files(dir: &camino::Utf8Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect()
}

/// The actual recording daemon entry point.
///
/// Acquires the record lock, captures audio + editor state + file diffs,
/// waits for stop/flush sentinels, transcribes, merges, and writes output.
///
/// When silence-based segmentation is enabled (the default), completed speech
/// segments are transcribed on the fly and their audio is freed. At stop/flush,
/// only the current in-progress segment needs transcription.
pub fn daemon(clock: Arc<dyn Clock>) -> anyhow::Result<()> {
    // Create a new session so the daemon survives if the parent's process
    // group is killed (e.g. Zed's task runner cleaning up after toggle exits).
    // Intentionally ignored: may fail if already session leader (e.g. run manually).
    let _ = nix::unistd::setsid();

    let cwd = camino::Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
        .unwrap_or_else(|_| camino::Utf8PathBuf::from("."));
    let config = Config::load(&cwd);
    let engine = config.engine.unwrap_or(Engine::Parakeet);
    let idle_timeout = config.idle_timeout();
    let model_path = config.model.unwrap_or_else(|| engine.default_model_path());
    let session_id = resolve_session(None);

    // Ensure daemon dir exists (creates cache_dir/daemon/ and parents)
    let dd = super::daemon_dir();
    fs::create_dir_all(&dd)?;

    // Acquire record lock (auto-removed on drop, even on error/panic)
    let _lock = lockfile::Lockfile::create(record_lock_path())
        .map_err(|e| anyhow::anyhow!("record lock already held: {e:?}"))?;

    // Best-effort PID+timestamp write for stale lock detection. This is not
    // atomic with the lock creation above: if the process is SIGKILL'd between
    // Lockfile::create and this write, the lock file will exist without a
    // PID, making is_lock_stale() return false (permanently stuck lock).
    // This is acceptable because:
    // - SIGTERM is caught by our signal handler and triggers clean shutdown
    //   (Lockfile::Drop removes the file).
    // - Only SIGKILL can cause the stuck state, and that's unrecoverable
    //   by design.
    let _ = fs::write(record_lock_path(), super::lock_file_content());

    // Best-effort cleanup: sentinels may not exist.
    let _ = fs::remove_file(stop_sentinel_path());
    let _ = fs::remove_file(flush_sentinel_path());
    let _ = fs::remove_file(pause_sentinel_path());
    let _ = fs::remove_file(yank_sentinel_path());

    // Set up audio capture, capture threads, and transcriber.
    // In test mode: stub sources, no cpal, no model loading.
    // Set up audio capture, capture threads, and transcriber.
    // In test mode: stub sources, no cpal, no model loading.
    let (capture, capture_config, transcriber): (
        Box<dyn audio::AudioSource>,
        capture::CaptureConfig,
        DeferredTranscriber,
    ) = if crate::test_mode::is_active() {
        use crate::test_mode::stubs::StubAudioSource;

        let audio = Box::new(StubAudioSource::new(16000, Arc::clone(&clock)));
        let (cap_config, stub_transcriber) = capture::CaptureConfig::test_mode(Arc::clone(&clock));
        let transcriber = DeferredTranscriber::preloaded(Box::new(stub_transcriber));
        (audio, cap_config, transcriber)
    } else {
        let audio = audio::start_capture()?;
        let cap_config = capture::CaptureConfig::production(Arc::clone(&clock));
        let transcriber = DeferredTranscriber::spawn(engine, model_path);
        (Box::new(audio), cap_config, transcriber)
    };

    let sample_rate = capture.sample_rate();

    // Start editor polling, file diff tracking, and external selection capture
    // on background threads. Pass None for cwd so paths stay absolute — filtering
    // is deferred to receive.
    let clipboard_enabled = config.clipboard_capture.unwrap_or(true);
    let clipboard_staging = super::clipboard_staging_dir(session_id.as_ref());
    let editor_events = capture::start(capture_config, None, clipboard_enabled, clipboard_staging)?;

    // Register SIGTERM handler so `kill <pid>` triggers a clean shutdown
    // (transcribe remaining audio and release the lock) instead of an abrupt exit.
    let terminated = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&terminated))?;

    // Intentionally ignored: chime failure should not abort recording.
    let _ = Chime::Start.play();

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

    let now = clock.now();
    let mut state = DaemonState {
        clock: clock.for_thread(),
        transcriber,
        audio_capture: Some(capture),
        editor_capture: Some(editor_events),
        terminated,
        silence_detector,
        buffered_chunks: Vec::new(),
        pre_transcribed: Vec::new(),
        period_start: now,
        time_base_secs: 0.0,
        last_drain: now,
        sample_rate,
        session_id,
        stop_sentinel: stop_sentinel_path(),
        flush_sentinel: flush_sentinel_path(),
        pause_sentinel: pause_sentinel_path(),
        yank_sentinel: yank_sentinel_path(),
        paused: false,
        pause_started_at: None,
        idle: false,
        idle_since: None,
        idle_timeout,
    };

    // In test mode, connect to the inject socket NOW — after all
    // initialization is complete. The harness interprets "daemon
    // connected" as "daemon ready to receive injections."
    if crate::test_mode::is_active() {
        crate::test_mode::connect();
    }

    loop {
        if !state.paused {
            state.ingest_chunks()?;
        }
        if state.check_terminated()? {
            break;
        }
        if state.check_yank()? {
            break;
        }
        state.check_stop()?;
        if state.check_flush()? {
            continue;
        }
        state.check_pause()?;
        if state.check_idle_timeout() {
            tracing::info!("Idle timeout reached, exiting.");
            break;
        }
        state
            .clock
            .sleep(Duration::from_millis(DAEMON_LOOP_POLL_MS));
    }

    // Best-effort cleanup: remove pause sentinel on any exit path.
    let _ = fs::remove_file(pause_sentinel_path());

    Ok(())
}
