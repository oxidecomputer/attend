//! End-to-end tests for the attend harness.
//!
//! These tests spawn a real daemon in test mode, drive it via CLI
//! subprocesses, and assert on observable outputs. Each test gets a
//! fresh isolated cache directory and inject socket.
//!
//! All processes are background: the harness spawns them, advances mock
//! time, and observes exits as trace events. No blocking waits for
//! specific children — this is the all-background execution model.
//!
//! Run with: `cargo nextest run --test e2e`
//!
//! # Note on property tests
//!
//! Property-based e2e tests are not included because each test case
//! spawns real OS processes, binds Unix sockets, and uses condvar-gated
//! mock clocks for time coordination. Running hundreds of randomly
//! generated inputs through this infrastructure would be prohibitively
//! slow and fragile.
//!
//! The invariants that proptest would verify (e.g., "flush boundaries
//! are disjoint", "merge is order-preserving") are tested at the unit
//! level where they belong:
//!   - `src/narrate/merge/tests/prop.rs` (event merge properties)
//!   - `src/hook/tests/prop.rs` (hook decision properties)

use attend_test_harness::{FileEntry, TestHarness};

/// Locate the attend binary built by cargo for this test run.
fn binary() -> String {
    env!("CARGO_BIN_EXE_attend").to_string()
}

/// Build activate-session hook stdin JSON.
fn activate_json(session_id: &str, cwd: &str) -> String {
    serde_json::json!({
        "session_id": session_id,
        "cwd": cwd,
        "prompt": "/attend",
    })
    .to_string()
}

/// Build collect-narration (pre-tool-use listen) hook stdin JSON.
fn collect_json(session_id: &str, cwd: &str, binary: &str) -> String {
    serde_json::json!({
        "session_id": session_id,
        "cwd": cwd,
        "tool_name": "Bash",
        "tool_input": {
            "command": format!("{binary} listen --wait --session {session_id}"),
        },
    })
    .to_string()
}

/// Activate a session via the user-prompt hook and wait for it to exit.
fn activate(h: &mut TestHarness, session_id: &str) {
    let cwd = h.cache_dir().to_owned();
    let id = h.spawn_with_stdin(
        &["hook", "user-prompt", "-a", "claude"],
        activate_json(session_id, cwd.as_str()).as_bytes(),
    );
    h.tick_until_exit(id);
}

/// Collect pending narration via the pre-tool-use hook and return stdout.
fn collect(h: &mut TestHarness, session_id: &str) -> String {
    let cwd = h.cache_dir().to_owned();
    let bin = h.binary().to_owned();
    let id = h.spawn_with_stdin(
        &["hook", "pre-tool-use", "-a", "claude"],
        collect_json(session_id, cwd.as_str(), bin.as_str()).as_bytes(),
    );
    let event = h.tick_until_exit(id);
    String::from_utf8(event.stdout).expect("non-UTF-8 hook output")
}

// =========================================================================
// Smoke tests (original 5)
// =========================================================================

/// Start recording, inject speech, stop, collect: the delivered
/// narration should contain the injected words.
#[test]
fn start_speak_stop_collect() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "sess-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    h.inject_speech("hello world from the harness", 2000);
    h.advance_time(500);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "sess-1");
    assert!(
        narration.contains("hello world from the harness"),
        "narration should contain injected speech:\n{narration}"
    );
}

/// Status reports that the daemon is recording, then idle after stop.
#[test]
fn status_shows_recording_state() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "sess-2");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);
    h.advance_time(500);

    let status = h.spawn(&["narrate", "status"]);
    let event = h.tick_until_exit(status);
    let output = String::from_utf8(event.stdout).expect("non-UTF-8 output");
    assert!(
        output.contains("Recording") || output.contains("recording"),
        "status should indicate recording:\n{output}"
    );

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let status = h.spawn(&["narrate", "status"]);
    let event = h.tick_until_exit(status);
    let output = String::from_utf8(event.stdout).expect("non-UTF-8 output");
    assert!(
        !output.contains("recording"),
        "status should not indicate recording after stop:\n{output}"
    );
}

/// Shell hook events staged during recording appear in collected
/// narration. The event may be scope-filtered (✂) if the shell's cwd
/// doesn't match the agent's cwd.
#[test]
fn shell_event_appears_in_narration() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "sess-3");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);
    h.advance_time(500);

    let shell = h.spawn(&[
        "shell-hook",
        "postexec",
        "--shell",
        "fish",
        "--command",
        "cargo test",
        "--exit-status",
        "0",
        "--duration",
        "1.5",
    ]);
    h.tick_until_exit(shell);
    h.advance_time(500);

    h.inject_speech("some words", 500);
    h.advance_time(500);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "sess-3");
    // The shell event appears either with full content or as a ✂ marker
    // (depends on whether the test runner's cwd is within the hook's scope).
    assert!(
        narration.contains("command") || narration.contains("✂"),
        "narration should contain the shell event:\n{narration}"
    );
}

/// Multiple speech injections across a recording period are all delivered.
#[test]
fn multiple_speech_injections() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "sess-5");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    h.inject_speech("first utterance", 1000);
    h.advance_time(500);
    h.inject_speech("second utterance", 1000);
    h.advance_time(500);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "sess-5");
    assert!(
        narration.contains("first utterance"),
        "narration should contain first utterance:\n{narration}"
    );
    assert!(
        narration.contains("second utterance"),
        "narration should contain second utterance:\n{narration}"
    );
}

/// Collecting when nothing is pending returns no narration content.
#[test]
fn collect_empty_when_no_narration() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "sess-6");

    let narration = collect(&mut h, "sess-6");
    assert!(
        !narration.contains("<narration>"),
        "no narration content should be delivered:\n{narration}"
    );
}

// =========================================================================
// Pause / resume
// =========================================================================

/// Pausing when already paused toggles back to recording (the pause
/// sentinel is removed). Resuming when already recording is a no-op.
///
/// Invariant: `narrate pause` is a toggle: it creates the pause sentinel
/// if absent, removes it if present. Double-pause creates then removes
/// the sentinel, returning the daemon to recording state.
#[test]
fn double_pause_toggles_back_to_recording() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "pause-2");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    h.inject_speech("before double pause", 1000);
    h.advance_time(500);

    // Pause twice (second removes the sentinel, resuming recording).
    let p1 = h.spawn(&["narrate", "pause"]);
    h.tick_until_exit(p1);
    h.advance_time(200);
    let p2 = h.spawn(&["narrate", "pause"]);
    h.tick_until_exit(p2);
    h.advance_time(200);

    // The second `narrate pause` toggled back to recording.
    h.inject_speech("after double pause toggle", 1000);
    h.advance_time(500);

    // Stop.
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "pause-2");
    assert!(
        narration.contains("before double pause"),
        "narration should contain pre-pause speech:\n{narration}"
    );
    assert!(
        narration.contains("after double pause toggle"),
        "narration should contain post-double-toggle speech:\n{narration}"
    );
}

// =========================================================================
// Flush (mid-recording delivery)
// =========================================================================

/// Flush writes pending narration without stopping the daemon.
///
/// Invariant: `narrate start` on an already-recording daemon triggers a
/// flush. Content before the flush is delivered as a pending narration
/// file. Content after the flush is captured in a new period and delivered
/// on stop.
#[test]
fn flush_delivers_mid_recording() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "flush-1");

    // Start recording (spawns daemon).
    let start = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(start);

    // First speech segment.
    h.inject_speech("segment one content", 1000);
    h.advance_time(500);

    // Flush: `narrate start` while already recording triggers flush.
    let flush = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(flush);
    // Let daemon process the flush sentinel.
    h.advance_time(200);

    // Collect: should have the first segment.
    let narration_1 = collect(&mut h, "flush-1");
    assert!(
        narration_1.contains("segment one content"),
        "first collect should contain flushed segment:\n{narration_1}"
    );

    // Second speech segment (after flush, daemon still recording).
    h.inject_speech("segment two content", 1000);
    h.advance_time(500);

    // Stop recording.
    let stop = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(stop);

    // Collect: should have the second segment.
    let narration_2 = collect(&mut h, "flush-1");
    assert!(
        narration_2.contains("segment two content"),
        "second collect should contain post-flush segment:\n{narration_2}"
    );
}

/// Flushing when nothing has been spoken produces no narration content.
///
/// Invariant: a flush with no captured content does not create a pending
/// narration file.
#[test]
fn flush_with_no_content_produces_nothing() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "flush-2");

    // Start recording.
    let start = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(start);
    h.advance_time(200);

    // Flush immediately (no speech injected).
    let flush = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(flush);
    h.advance_time(200);

    // Collect: nothing pending.
    let narration = collect(&mut h, "flush-2");
    assert!(
        !narration.contains("<narration>"),
        "flush with no content should not produce narration:\n{narration}"
    );

    // Stop (clean up daemon).
    let stop = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(stop);
}

/// A flush followed by a stop produces two separate narration deliveries.
///
/// Invariant: the flush delivers the first batch of content. The stop
/// delivers the second batch. Both are independently collectible.
#[test]
fn flush_then_stop_produces_two_deliveries() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "flush-stop-1");

    let start = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(start);

    h.inject_speech("batch one", 1000);
    h.advance_time(500);

    // Flush.
    let flush = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(flush);
    h.advance_time(200);

    // Collect after flush.
    let batch_1 = collect(&mut h, "flush-stop-1");
    assert!(
        batch_1.contains("batch one"),
        "first collect should contain flushed content:\n{batch_1}"
    );

    h.inject_speech("batch two", 1000);
    h.advance_time(500);

    // Stop.
    let stop = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(stop);

    let batch_2 = collect(&mut h, "flush-stop-1");
    assert!(
        batch_2.contains("batch two"),
        "second collect should contain post-flush content:\n{batch_2}"
    );
    assert!(
        !batch_2.contains("batch one"),
        "second collect should not re-deliver flushed content:\n{batch_2}"
    );
}

/// Three consecutive flushes produce three independently collectible
/// narration deliveries.
///
/// Invariant: each flush writes only the content captured since the
/// previous flush. Content is never duplicated across flush boundaries,
/// and the daemon continues recording throughout.
#[test]
fn multiple_sequential_flushes() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "multi-flush-1");

    let start = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(start);

    // Segment 1.
    h.inject_speech("alpha segment", 1000);
    h.advance_time(500);

    let flush1 = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(flush1);
    h.advance_time(200);

    let batch_1 = collect(&mut h, "multi-flush-1");
    assert!(
        batch_1.contains("alpha segment"),
        "first flush should contain alpha:\n{batch_1}"
    );

    // Segment 2.
    h.inject_speech("beta segment", 1000);
    h.advance_time(500);

    let flush2 = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(flush2);
    h.advance_time(200);

    let batch_2 = collect(&mut h, "multi-flush-1");
    assert!(
        batch_2.contains("beta segment"),
        "second flush should contain beta:\n{batch_2}"
    );
    assert!(
        !batch_2.contains("alpha segment"),
        "second flush should not re-deliver alpha:\n{batch_2}"
    );

    // Segment 3.
    h.inject_speech("gamma segment", 1000);
    h.advance_time(500);

    let flush3 = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(flush3);
    h.advance_time(200);

    let batch_3 = collect(&mut h, "multi-flush-1");
    assert!(
        batch_3.contains("gamma segment"),
        "third flush should contain gamma:\n{batch_3}"
    );
    assert!(
        !batch_3.contains("alpha segment"),
        "third flush should not contain alpha:\n{batch_3}"
    );
    assert!(
        !batch_3.contains("beta segment"),
        "third flush should not contain beta:\n{batch_3}"
    );

    // Stop.
    let stop = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(stop);
}

// =========================================================================
// Idle timeout
// =========================================================================

/// A daemon that enters idle state exits after the idle timeout elapses.
///
/// Invariant: after stop (which puts the daemon into idle), advancing mock
/// time past the 5-minute default idle timeout causes the daemon to exit
/// cleanly. The daemon checks `check_idle_timeout()` each loop iteration
/// (100ms poll interval).
#[test]
fn idle_timeout_stops_daemon() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "idle-1");

    // Start recording.
    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);
    h.advance_time(200);

    // Stop recording (daemon enters idle with model loaded).
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    // Daemon is now idle. Default idle timeout is 5 minutes (300s).
    // Advance past the timeout. The daemon polls at 100ms intervals,
    // so we advance in a chunk large enough to exceed 5 minutes,
    // then give the daemon one more tick to process the exit.
    h.advance_time(310_000);

    // Give the daemon additional ticks to process the idle timeout
    // check and exit. The advance above jumped the clock, but the
    // daemon needs one more loop iteration to call check_idle_timeout().
    for _ in 0..10 {
        h.advance_time(100);
    }

    assert!(
        !h.has_daemon(),
        "daemon should have exited after idle timeout"
    );
}

// =========================================================================
// Editor state interleaved with speech
// =========================================================================

/// Editor snapshots injected during recording appear in the narration
/// alongside speech.
///
/// Invariant: injecting editor state (file path + cursor position) and
/// speech during the same recording period produces narration that
/// contains both the file reference (possibly ✂-snipped if CWD doesn't
/// match) and the spoken words.
#[test]
fn editor_state_interleaved_with_speech() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "editor-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    // Inject editor state: a file with cursor position.
    h.inject_editor_state(vec![FileEntry::with_cursor("src/main.rs", 42, 1)]);
    h.advance_time(200);

    // Inject speech referencing the file.
    h.inject_speech("look at this function in main", 2000);
    h.advance_time(500);

    // Stop.
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "editor-1");
    // Editor state appears in narration. The file path may be snipped (✂)
    // if the test CWD doesn't match the file path. Either way, the editor
    // capture event is present.
    assert!(
        narration.contains("main.rs") || narration.contains("✂"),
        "narration should contain editor file path or snip marker:\n{narration}"
    );
    // Speech may be fragmented across event boundaries in the rendered
    // output. Check for key words individually.
    assert!(
        narration.contains("look") && narration.contains("main"),
        "narration should contain speech words:\n{narration}"
    );
}

/// Multiple editor snapshots are captured and the latest state is reflected.
///
/// Invariant: when editor state changes during recording, the narration
/// includes references to files from each snapshot (possibly snipped).
#[test]
fn editor_state_updates_reflected() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "editor-2");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    // First editor state.
    h.inject_editor_state(vec![FileEntry::with_cursor("src/lib.rs", 10, 1)]);
    h.advance_time(200);

    h.inject_speech("first file", 1000);
    h.advance_time(500);

    // Switch to a different file.
    h.inject_editor_state(vec![FileEntry::with_cursor("src/config.rs", 5, 1)]);
    h.advance_time(200);

    h.inject_speech("second file", 1000);
    h.advance_time(500);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "editor-2");
    // File paths may be snipped, but the editor events are present.
    // Check that both speech segments and some editor event markers exist.
    assert!(
        narration.contains("first") && narration.contains("second"),
        "narration should contain both speech segments:\n{narration}"
    );
    // At least one editor event is present (may be ✂-snipped).
    assert!(
        narration.contains("lib.rs") || narration.contains("config.rs") || narration.contains("✂"),
        "narration should contain editor events:\n{narration}"
    );
}

// =========================================================================
// Session handoff / displacement
// =========================================================================

/// When session B activates while session A is the listener, session A
/// is marked as displaced and subsequent hooks for A report the move.
///
/// Invariant: activating a new session via the user-prompt hook displaces
/// the previous listener. A subsequent hook invocation for the displaced
/// session includes "session" in its guidance output.
#[test]
fn session_handoff_displacement() {
    let mut h = TestHarness::new(binary());

    // Session A activates.
    activate(&mut h, "handoff-a");

    // Session B activates (displaces A).
    activate(&mut h, "handoff-b");

    // Collect from session A: should see displacement notice.
    let output_a = collect(&mut h, "handoff-a");
    // The hook system reports guidance when the session has been displaced.
    // The exact wording includes "different session" or "session" context.
    assert!(
        output_a.contains("session")
            || output_a.contains("moved")
            || output_a.contains("displaced"),
        "displaced session A should see handoff notice:\n{output_a}"
    );
}

// =========================================================================
// Stale lock recovery
// =========================================================================

/// A stale lock file (PID that no longer exists) is recovered on toggle.
///
/// Invariant: if a lock file exists but its PID is dead, `narrate toggle`
/// cleans up the stale lock and spawns a fresh daemon. Recording then
/// works normally.
#[test]
fn stale_lock_recovery() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "stale-1");

    // Write a fake lock file with a PID that doesn't exist.
    // PID 999999 is extremely unlikely to exist.
    let daemon_dir = h.cache_dir().join("daemon");
    std::fs::create_dir_all(&daemon_dir).expect("failed to create daemon dir");
    std::fs::write(daemon_dir.join("lock"), "999999").expect("failed to write stale lock");

    // Toggle should recover the stale lock and start a new daemon.
    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    // Recording should work: inject speech and stop.
    h.inject_speech("recovered from stale lock", 1000);
    h.advance_time(500);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "stale-1");
    assert!(
        narration.contains("recovered from stale lock"),
        "narration should contain speech after stale lock recovery:\n{narration}"
    );
}

// =========================================================================
// Silence-based segmentation
// =========================================================================

/// Silence gaps between speech are detected and segment boundaries are
/// preserved in the output.
///
/// Invariant: when speech is separated by a silence gap exceeding the
/// segmentation threshold (default 5s), the transcriber processes them
/// as distinct segments. Both segments appear in the final output.
#[test]
fn silence_based_segmentation() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "silence-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    // First speech segment.
    h.inject_speech("segment alpha", 2000);
    h.advance_time(1000);

    // Inject silence exceeding the 5-second default threshold.
    h.inject_silence(6000);
    h.advance_time(7000);

    // Second speech segment.
    h.inject_speech("segment beta", 2000);
    h.advance_time(1000);

    // Stop.
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "silence-1");
    assert!(
        narration.contains("segment alpha"),
        "narration should contain first speech segment:\n{narration}"
    );
    assert!(
        narration.contains("segment beta"),
        "narration should contain second speech segment:\n{narration}"
    );
}

// =========================================================================
// Resume from idle starts fresh session
// =========================================================================

/// Resuming from idle (after stop) starts a fresh recording period.
///
/// Invariant: after toggle-off (stop, daemon enters idle) and toggle-on
/// (resume from idle), the new recording period is independent. Speech
/// from the new period is collected separately from the stopped period.
#[test]
fn resume_from_idle_starts_fresh_period() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "idle-resume-1");

    // First recording period.
    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    h.inject_speech("first period speech", 1000);
    h.advance_time(500);

    // Stop (daemon enters idle).
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    // Collect first period.
    let narration_1 = collect(&mut h, "idle-resume-1");
    assert!(
        narration_1.contains("first period speech"),
        "first period should contain its speech:\n{narration_1}"
    );

    // Resume from idle (toggle on again, daemon resumes).
    let toggle_on2 = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on2);

    h.inject_speech("second period speech", 1000);
    h.advance_time(500);

    // Stop again.
    let toggle_off2 = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off2);

    // Collect second period.
    let narration_2 = collect(&mut h, "idle-resume-1");
    assert!(
        narration_2.contains("second period speech"),
        "second period should contain its speech:\n{narration_2}"
    );
    // The first period's speech should NOT reappear (it was already collected).
    assert!(
        !narration_2.contains("first period speech"),
        "second collect should not contain first period speech:\n{narration_2}"
    );
}

// =========================================================================
// Stop when not recording
// =========================================================================

/// Stopping when not recording is a no-op that does not error.
///
/// Invariant: `narrate stop` with no lock file exits cleanly without
/// creating any sentinel files.
#[test]
fn stop_when_not_recording_is_noop() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "noop-1");

    // Stop without ever starting.
    let stop = h.spawn(&["narrate", "stop"]);
    let event = h.tick_until_exit(stop);
    assert_eq!(
        event.exit_code, 0,
        "stop when not recording should exit cleanly"
    );
}

// =========================================================================
// External selection capture
// =========================================================================

/// External selections (e.g., browser text) injected during recording
/// appear in the narration output.
///
/// Invariant: external selection events are captured alongside speech
/// and appear in the merged narration output.
#[test]
fn external_selection_appears_in_narration() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "ext-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    // Inject an external selection (simulating browser/app selection).
    h.inject_external_selection("Safari", "selected documentation text");
    h.advance_time(200);

    h.inject_speech("referencing the docs", 1000);
    h.advance_time(500);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "ext-1");
    assert!(
        narration.contains("selected documentation text"),
        "narration should contain external selection:\n{narration}"
    );
    // Speech words may be fragmented around the selection in the rendered
    // output. Check for individual key words.
    assert!(
        narration.contains("referencing") && narration.contains("docs"),
        "narration should contain speech words:\n{narration}"
    );
}

// =========================================================================
// Yank writes to yanked directory
// =========================================================================

/// `narrate yank` when not recording exits cleanly (no-op).
///
/// Invariant: yank without an active daemon returns immediately with
/// exit code 0 and does not create any sentinel files.
///
/// Note: a full yank-while-recording e2e test is not feasible with the
/// current harness because yank calls `CaptureHandle::collect()` which
/// joins the clipboard thread. The clipboard thread sleeps on MockClock,
/// and no time advances occur during `collect()`, causing a deadlock.
/// The yank behavior (write to yanked dir, exit daemon) is tested at
/// the unit level in `src/narrate/tests.rs`.
#[test]
fn yank_when_not_recording_is_noop() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "yank-1");

    // Yank without ever starting: should be a no-op.
    let yank = h.spawn(&["narrate", "yank"]);
    let event = h.tick_until_exit(yank);
    assert_eq!(
        event.exit_code, 0,
        "yank when not recording should exit cleanly"
    );
}

// =========================================================================
// Start on idle daemon resumes recording
// =========================================================================

/// `narrate start` on an idle daemon resumes recording without spawning
/// a new daemon.
///
/// Invariant: when the daemon is idle (after stop), `narrate start`
/// removes the pause sentinel, causing the daemon to resume. The resumed
/// session captures new speech independently. This is distinct from
/// toggle-on (which also resumes) in that `start` on a recording daemon
/// triggers flush rather than stop.
#[test]
fn start_on_idle_daemon_resumes() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "start-idle-1");

    // Start recording.
    let start = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(start);

    h.inject_speech("first period", 1000);
    h.advance_time(500);

    // Stop (daemon enters idle).
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    // Collect first period.
    let narration_1 = collect(&mut h, "start-idle-1");
    assert!(
        narration_1.contains("first period"),
        "first period should contain its speech:\n{narration_1}"
    );

    // `narrate start` on idle daemon should resume (not spawn new).
    let start_again = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(start_again);

    h.inject_speech("resumed via start", 1000);
    h.advance_time(500);

    // Stop again.
    let toggle_off2 = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off2);

    let narration_2 = collect(&mut h, "start-idle-1");
    assert!(
        narration_2.contains("resumed via start"),
        "second period should contain speech after start-on-idle:\n{narration_2}"
    );
    assert!(
        !narration_2.contains("first period"),
        "second collect should not contain first period speech:\n{narration_2}"
    );
}

// =========================================================================
// Clipboard text changes appear in narration
// =========================================================================

/// Clipboard text injected during recording appears in the narration.
///
/// Invariant: when the clipboard content changes during an active
/// recording period, the change is captured as a clipboard selection
/// event and included in the merged narration output.
#[test]
fn clipboard_text_appears_in_narration() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "clip-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    // Inject clipboard content change.
    h.inject_clipboard("clipboard snippet from docs");
    // The clipboard polling thread runs at 500ms intervals; advance
    // enough for it to see the change.
    h.advance_time(600);

    h.inject_speech("check the clipboard now", 1000);
    h.advance_time(500);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "clip-1");
    assert!(
        narration.contains("clipboard snippet from docs"),
        "narration should contain clipboard text:\n{narration}"
    );
    // Speech words may be fragmented around the clipboard event.
    // Check key words individually.
    assert!(
        narration.contains("check") && narration.contains("clipboard"),
        "narration should contain speech words:\n{narration}"
    );
}

// =========================================================================
// Toggle while paused resumes
// =========================================================================

/// Toggle while paused resumes recording (daemon sees lock + pause sentinel).
///
/// Invariant: when the daemon is paused (user-initiated), toggle
/// sees `lock + pause sentinel` and removes the sentinel, causing
/// the daemon to resume recording. Speech injected after the resume
/// appears in the output.
#[test]
fn toggle_while_paused_resumes() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "toggle-pause-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    h.inject_speech("before pause content", 1000);
    h.advance_time(500);

    // Pause.
    let pause = h.spawn(&["narrate", "pause"]);
    h.tick_until_exit(pause);
    h.advance_time(200);

    // Toggle while paused: resumes (sees lock + pause sentinel).
    let toggle = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle);
    h.advance_time(200);

    // Daemon should be recording again. Inject speech.
    h.inject_speech("after toggle resume", 1000);
    h.advance_time(500);

    // Stop recording.
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration = collect(&mut h, "toggle-pause-1");
    assert!(
        narration.contains("before pause content"),
        "narration should contain pre-pause speech:\n{narration}"
    );
    assert!(
        narration.contains("after toggle resume"),
        "narration should contain post-resume speech:\n{narration}"
    );
}

// =========================================================================
// Stop while paused is no-op
// =========================================================================

/// `narrate stop` when the daemon is paused (pause sentinel exists)
/// is a no-op.
///
/// Invariant: the `stop()` function checks `pause_sentinel_path().exists()`
/// and returns early if true (the daemon is already idle or paused).
/// This prevents races between the CLI stop and the daemon's internal
/// state. To stop a paused daemon, resume first then stop.
#[test]
fn stop_while_paused_is_noop() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "stop-pause-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    h.inject_speech("buffered before pause", 1000);
    h.advance_time(500);

    // Pause.
    let pause = h.spawn(&["narrate", "pause"]);
    h.tick_until_exit(pause);
    h.advance_time(200);

    // `narrate stop` while paused: returns immediately (no-op).
    let stop = h.spawn(&["narrate", "stop"]);
    let event = h.tick_until_exit(stop);
    assert_eq!(event.exit_code, 0, "stop while paused should exit cleanly");

    // Content is NOT delivered yet (daemon still alive, content buffered).
    let narration = collect(&mut h, "stop-pause-1");
    assert!(
        !narration.contains("<narration>"),
        "stop while paused should not deliver content:\n{narration}"
    );

    // Resume and then stop properly to get the content.
    let resume = h.spawn(&["narrate", "pause"]);
    h.tick_until_exit(resume);
    h.advance_time(200);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    let narration_after = collect(&mut h, "stop-pause-1");
    assert!(
        narration_after.contains("buffered before pause"),
        "content should be delivered after resume+stop:\n{narration_after}"
    );
}

// =========================================================================
// Daemon survives repeated flush cycles
// =========================================================================

/// The daemon stays alive through a flush-collect-speak cycle repeated
/// multiple times.
///
/// Invariant: the daemon process does not exit or become unresponsive
/// after repeated flush cycles. Each cycle produces fresh content and
/// the daemon correctly resets its internal state.
#[test]
fn daemon_survives_repeated_flush_cycles() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "cycle-1");

    let start = h.spawn(&["narrate", "start"]);
    h.tick_until_exit(start);

    for i in 0..5 {
        let content = format!("cycle {i} speech");
        h.inject_speech(&content, 1000);
        h.advance_time(500);

        let flush = h.spawn(&["narrate", "start"]);
        h.tick_until_exit(flush);
        h.advance_time(200);

        let narration = collect(&mut h, "cycle-1");
        assert!(
            narration.contains(&content),
            "cycle {i} should deliver its content:\n{narration}"
        );
    }

    // Daemon should still be alive.
    assert!(
        h.has_daemon(),
        "daemon should survive repeated flush cycles"
    );

    // Clean stop.
    let stop = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(stop);
}

// =========================================================================
// Idle daemon resumes with fresh session identity
// =========================================================================

/// When the daemon resumes from idle, it re-resolves the session ID.
///
/// Invariant: if session B activates while the daemon is idle (session A
/// was active), the resumed recording targets session B. Content from
/// the new period is collected under session B, not session A.
#[test]
fn idle_resume_re_resolves_session() {
    let mut h = TestHarness::new(binary());

    // Session A activates.
    activate(&mut h, "resolve-a");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    h.inject_speech("session a content", 1000);
    h.advance_time(500);

    // Stop (daemon enters idle).
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    // Collect session A content.
    let narration_a = collect(&mut h, "resolve-a");
    assert!(
        narration_a.contains("session a content"),
        "session A should have its content:\n{narration_a}"
    );

    // Session B activates while daemon is idle.
    activate(&mut h, "resolve-b");

    // Resume from idle (toggle sees lock + pause sentinel).
    let toggle_on2 = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on2);

    h.inject_speech("session b content", 1000);
    h.advance_time(500);

    let toggle_off2 = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off2);

    // Collect under session B.
    let narration_b = collect(&mut h, "resolve-b");
    assert!(
        narration_b.contains("session b content"),
        "session B should have its content after idle resume:\n{narration_b}"
    );
}

// =========================================================================
// Stop when already idle is no-op
// =========================================================================

/// `narrate stop` when the daemon is already idle (after a previous stop)
/// is a no-op: it does not create a stop sentinel or produce content.
///
/// Invariant: calling stop on an idle daemon exits cleanly without
/// creating duplicate sentinels or producing empty narration files.
#[test]
fn stop_when_already_idle_is_noop() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "idle-stop-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);
    h.advance_time(200);

    // Stop (daemon enters idle).
    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    // Stop again while idle: should be no-op.
    let stop = h.spawn(&["narrate", "stop"]);
    let event = h.tick_until_exit(stop);
    assert_eq!(
        event.exit_code, 0,
        "stop when already idle should exit cleanly"
    );

    // No spurious content.
    let narration = collect(&mut h, "idle-stop-1");
    assert!(
        !narration.contains("<narration>"),
        "stop when idle should not produce narration:\n{narration}"
    );
}

// =========================================================================
// Collect is idempotent when no new content
// =========================================================================

/// Collecting twice without new content returns nothing on the second call.
///
/// Invariant: the hook system delivers pending narration files exactly
/// once. After collection, the files are archived (moved out of pending).
/// A subsequent collect with no new content returns empty.
#[test]
fn collect_is_idempotent() {
    let mut h = TestHarness::new(binary());

    activate(&mut h, "idempotent-1");

    let toggle_on = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_on);

    h.inject_speech("delivered once", 1000);
    h.advance_time(500);

    let toggle_off = h.spawn(&["narrate", "toggle"]);
    h.tick_until_exit(toggle_off);

    // First collect: should have content.
    let narration_1 = collect(&mut h, "idempotent-1");
    assert!(
        narration_1.contains("delivered once"),
        "first collect should contain content:\n{narration_1}"
    );

    // Second collect: should be empty (content already delivered).
    let narration_2 = collect(&mut h, "idempotent-1");
    assert!(
        !narration_2.contains("delivered once"),
        "second collect should not re-deliver content:\n{narration_2}"
    );
}
