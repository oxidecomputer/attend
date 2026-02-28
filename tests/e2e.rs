//! End-to-end smoke tests for the attend harness.
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

use attend_test_harness::TestHarness;

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
