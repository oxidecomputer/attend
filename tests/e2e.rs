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

    // Activate a session (simulates /attend).
    h.activate_session("sess-1");

    // Start recording.
    h.toggle();

    // Inject speech and advance time for the daemon to process.
    h.inject_speech("hello world from the harness", 2000);
    h.advance_time(500);

    // Stop recording.
    h.toggle();

    // Collect narration via the PreToolUse hook.
    let narration = h.collect("sess-1");

    assert!(
        narration.contains("hello world from the harness"),
        "narration should contain injected speech:\n{narration}"
    );
}
