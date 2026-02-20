//! Recording daemon: captures audio, editor state, and file diffs.
//!
//! The recorder spawns as a detached child process via `_record-daemon`.
//! The parent (toggle/start) exits immediately so the hotkey returns fast.
//! The daemon records until a stop sentinel file appears, then transcribes,
//! merges all streams, and writes the result as a pending narration file.

use std::fs;
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

/// Grace period after spawning the daemon for it to acquire the lock (ms).
const DAEMON_STARTUP_GRACE_MS: u64 = 200;

/// Main daemon loop poll interval (ms).
const DAEMON_LOOP_POLL_MS: u64 = 100;

/// Number of recent words to feed as transcription context.
const TRANSCRIPTION_CONTEXT_WORDS: usize = 50;

/// Minimum remaining audio duration (seconds) worth transcribing at stop/flush.
const MIN_TRANSCRIPTION_DURATION_SECS: f64 = 0.5;

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
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("narrate").arg("_record-daemon");

    // Detach stdio so the daemon doesn't hold the parent's descriptors.
    // The daemon calls setsid() at startup to create its own session.
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    cmd.spawn()?;

    // Give the daemon a moment to acquire the lock and start audio
    thread::sleep(Duration::from_millis(DAEMON_STARTUP_GRACE_MS));

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
    #[cfg(unix)]
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

    // Best-effort PID write for stale lock detection (lock already held).
    let _ = fs::write(record_lock_path(), std::process::id().to_string());

    // Best-effort cleanup: sentinels may not exist.
    let _ = fs::remove_file(stop_sentinel_path());
    let _ = fs::remove_file(flush_sentinel_path());

    // Preload model (blocks until ready — must complete before first transcription)
    let mut transcriber = engine.preload(&model_path)?;

    // Intentionally ignored: chime failure should not abort recording.
    let _ = audio::play_chime(true);

    // Start audio capture
    let capture = audio::start_capture()?;
    let sample_rate = capture.sample_rate();

    // Start editor polling and file diff tracking on background threads.
    // Pass None for cwd so paths stay absolute — filtering is deferred to receive.
    let editor_events = capture::start(None)?;

    // Silence-based segmentation (0 disables).
    let silence_secs = config.silence_duration.unwrap_or(5.0);
    let mut silence_detector = if silence_secs > 0.0 {
        Some(SilenceDetector::new(
            sample_rate,
            Duration::from_secs_f64(silence_secs),
        ))
    } else {
        None
    };

    // Chunks buffered since the last on-the-fly transcription (or period start).
    let mut buffered_chunks: Vec<AudioChunk> = Vec::new();
    // Words transcribed on the fly during this period.
    let mut pre_transcribed: Vec<Word> = Vec::new();
    // When the current period started (for computing word offsets).
    let mut period_start = Instant::now();

    // Track time base for word timestamp offsets across flushes
    let mut time_base_secs = 0.0_f64;
    let mut last_drain = Instant::now();

    let stop_sentinel = stop_sentinel_path();
    let flush_sentinel = flush_sentinel_path();

    loop {
        // 1. Ingest new chunks from the capture stream.
        for chunk in capture.take_chunks() {
            if let Some(ref mut detector) = silence_detector
                && let Some(silence_start) = detector.feed(&chunk)
            {
                buffered_chunks.push(chunk);

                // Partition: chunks whose instant < silence_start are speech.
                let split = buffered_chunks.partition_point(|c| c.instant < silence_start);
                let speech: Vec<_> = buffered_chunks.drain(..split).collect();
                // Discard trailing silence chunks.
                buffered_chunks.clear();
                detector.reset();

                if !speech.is_empty() {
                    transcribe_segment(
                        &mut *transcriber,
                        &speech,
                        sample_rate,
                        period_start,
                        &mut pre_transcribed,
                    )?;
                }
                continue;
            }
            buffered_chunks.push(chunk);
        }

        // 2. Check for stop sentinel → transcribe remaining + combine + write.
        if stop_sentinel.exists() {
            let _ = audio::play_chime(false); // Intentionally ignored: chime failure non-fatal
            // Grab any final chunks that arrived after the last take_chunks().
            let recording = capture.stop();
            buffered_chunks.extend(recording.chunks);
            let _ = fs::remove_file(&stop_sentinel); // Best-effort cleanup

            let (editor_snapshots, file_diffs) = editor_events.collect();
            transcribe_and_write(
                &mut *transcriber,
                std::mem::take(&mut buffered_chunks),
                sample_rate,
                std::mem::take(&mut pre_transcribed),
                period_start,
                editor_snapshots,
                file_diffs,
                time_base_secs,
                session_id.as_ref(),
            )?;
            break;
        }

        // 3. Check for flush sentinel → transcribe remaining, write, reset state.
        if flush_sentinel.exists() {
            let _ = audio::play_flush_chime(); // Intentionally ignored: chime failure non-fatal
            let recording = capture.drain();
            buffered_chunks.extend(recording.chunks);
            let elapsed = last_drain.elapsed().as_secs_f64();

            let (editor_snapshots, file_diffs) = editor_events.drain();
            transcribe_and_write(
                &mut *transcriber,
                std::mem::take(&mut buffered_chunks),
                sample_rate,
                std::mem::take(&mut pre_transcribed),
                period_start,
                editor_snapshots,
                file_diffs,
                time_base_secs,
                session_id.as_ref(),
            )?;

            time_base_secs += elapsed;
            last_drain = Instant::now();
            period_start = Instant::now();

            if let Some(ref mut detector) = silence_detector {
                detector.reset();
            }

            // Acknowledge flush by deleting sentinel (best-effort)
            let _ = fs::remove_file(&flush_sentinel);
            continue;
        }

        // 4. Sleep before next poll.
        thread::sleep(Duration::from_millis(DAEMON_LOOP_POLL_MS));
    }

    Ok(())
}

/// Transcribe a completed speech segment on the fly and append to pre_transcribed.
fn transcribe_segment(
    transcriber: &mut dyn super::transcribe::Transcriber,
    speech_chunks: &[AudioChunk],
    sample_rate: u32,
    period_start: Instant,
    pre_transcribed: &mut Vec<Word>,
) -> anyhow::Result<()> {
    let offset = speech_chunks[0]
        .instant
        .duration_since(period_start)
        .as_secs_f64();
    let samples_16k = audio::resample(
        &audio::flatten_chunks(speech_chunks),
        sample_rate,
        TRANSCRIPTION_SAMPLE_RATE,
    )?;

    tracing::debug!(
        duration_secs = samples_16k.len() as f64 / TRANSCRIPTION_SAMPLE_RATE as f64,
        offset_secs = offset,
        "Transcribing speech segment on the fly"
    );

    let words = transcriber.transcribe(&samples_16k)?;
    for w in &words {
        pre_transcribed.push(Word {
            start_secs: w.start_secs + offset,
            end_secs: w.end_secs + offset,
            text: w.text.clone(),
        });
    }

    // Feed context for next segment (Whisper uses it, Parakeet ignores).
    let ctx: String = pre_transcribed
        .iter()
        .rev()
        .take(TRANSCRIPTION_CONTEXT_WORDS)
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    transcriber.set_context(&ctx);

    Ok(())
}

/// Transcribe remaining audio, combine with pre-transcribed words, merge with
/// editor events, and write the pending file as JSON.
#[allow(clippy::too_many_arguments)]
fn transcribe_and_write(
    transcriber: &mut dyn super::transcribe::Transcriber,
    remaining_chunks: Vec<AudioChunk>,
    sample_rate: u32,
    pre_transcribed: Vec<Word>,
    period_start: Instant,
    editor_snapshots: Vec<Event>,
    file_diffs: Vec<Event>,
    time_base_secs: f64,
    session_id: Option<&SessionId>,
) -> anyhow::Result<()> {
    let mut all_words = pre_transcribed;

    // Transcribe any remaining buffered audio (the in-progress segment).
    if !remaining_chunks.is_empty() {
        let remaining_samples = audio::flatten_chunks(&remaining_chunks);
        let remaining_duration = remaining_samples.len() as f64 / sample_rate as f64;

        if remaining_duration >= MIN_TRANSCRIPTION_DURATION_SECS {
            let offset = remaining_chunks[0]
                .instant
                .duration_since(period_start)
                .as_secs_f64();

            tracing::info!(
                duration_secs = remaining_duration,
                "Transcribing remaining audio..."
            );

            let samples_16k =
                audio::resample(&remaining_samples, sample_rate, TRANSCRIPTION_SAMPLE_RATE)?;
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

    if all_words.is_empty() && editor_snapshots.is_empty() && file_diffs.is_empty() {
        tracing::debug!("No content captured, discarding.");
        return Ok(());
    }

    let mut events: Vec<Event> = Vec::new();

    for word in &all_words {
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
        tracing::debug!("No content captured after merge, discarding.");
        return Ok(());
    }

    let json = serde_json::to_string(&events)?;

    let ts = utc_now().replace(':', "-");
    if let Some(sid) = session_id {
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

    Ok(())
}
