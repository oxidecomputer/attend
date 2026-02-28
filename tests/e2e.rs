//! End-to-end smoke tests for the attend harness.
//!
//! These tests spawn a real daemon in test mode, drive it via CLI
//! subprocesses, and assert on observable outputs. Each test gets a
//! fresh isolated cache directory and inject socket.
//!
//! Run with: `cargo nextest run --test e2e`

use attend_test_harness::TestHarness;

/// Locate the attend binary built by cargo for this test run.
fn binary() -> String {
    env!("CARGO_BIN_EXE_attend").to_string()
}

/// Start recording, inject speech, stop, collect: the delivered
/// narration should contain the injected words.
#[test]
fn start_speak_stop_collect() {
    let mut h = TestHarness::new(binary());

    h.activate_session("sess-1");
    h.toggle();

    h.inject_speech("hello world from the harness", 2000);
    h.advance_time(500);

    h.toggle();

    let narration = h.collect("sess-1");
    assert!(
        narration.contains("hello world from the harness"),
        "narration should contain injected speech:\n{narration}"
    );
}

/// Status reports that the daemon is recording, then idle after stop.
///
/// Flaky: `narrate status` has 0 mock-clock waiters (purely synchronous),
/// so ACK-based ticks complete in microseconds. `yield_now()` between ticks
/// doesn't reliably give the daemon subprocess enough wall-clock time to
/// start and connect before the toggle command exits. The all-background
/// execution model (Phase 0 item 6) eliminates this class of race by
/// replacing `wait_child_ticking` with tick-settle-observe.
#[test]
#[ignore = "flaky: daemon startup races wait_child_ticking scheduling"]
fn status_shows_recording_state() {
    let mut h = TestHarness::new(binary());

    h.activate_session("sess-2");
    h.toggle();
    h.advance_time(500);

    let status = h.status();
    assert!(
        status.contains("Recording") || status.contains("recording"),
        "status should indicate recording:\n{status}"
    );

    h.toggle();

    let status = h.status();
    assert!(
        !status.contains("recording"),
        "status should not indicate recording after stop:\n{status}"
    );
}

/// Shell hook events staged during recording appear in collected
/// narration. The event may be scope-filtered (✂) if the shell's cwd
/// doesn't match the agent's cwd.
#[test]
fn shell_event_appears_in_narration() {
    let mut h = TestHarness::new(binary());

    h.activate_session("sess-3");
    h.toggle();
    h.advance_time(500);

    h.shell_event("fish", "cargo test", 0, 1.5);
    h.advance_time(500);

    h.inject_speech("some words", 500);
    h.advance_time(500);

    h.toggle();

    let narration = h.collect("sess-3");
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

    h.activate_session("sess-5");
    h.toggle();

    h.inject_speech("first utterance", 1000);
    h.advance_time(500);
    h.inject_speech("second utterance", 1000);
    h.advance_time(500);

    h.toggle();

    let narration = h.collect("sess-5");
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

    h.activate_session("sess-6");

    let narration = h.collect("sess-6");
    assert!(
        !narration.contains("<narration>"),
        "no narration content should be delivered:\n{narration}"
    );
}
