use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use camino::Utf8PathBuf;

use super::super::*;
use crate::agent::Agent;
use crate::narrate::merge::Event;
use crate::state::{self, EditorState, SessionId};

/// Convert seconds to a UTC timestamp (for test brevity).
fn ts(secs: f64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds((secs * 1000.0) as i64)
}

/// Monotonic counter for unique pending file names across test threads.
static PENDING_SEQ: AtomicU64 = AtomicU64::new(0);

/// What check_narration communicated back to the agent.
#[derive(Debug)]
pub(super) enum Outcome {
    /// `agent.attend_result` was called.
    Decision(HookDecision),
    /// `agent.deliver_narration` was called with this content.
    Narration(String),
    /// `agent.attend_activate` was called (auto-claim).
    Activation,
}

/// Mock agent that records hook output for assertion.
///
/// The `input` field is set by the harness before each call so
/// `parse_hook_input` returns the right session/tool context.
pub(super) struct MockAgent {
    input: Mutex<HookInput>,
    outcome: Mutex<Option<Outcome>>,
}

impl MockAgent {
    fn new(input: HookInput) -> Self {
        Self {
            input: Mutex::new(input),
            outcome: Mutex::new(None),
        }
    }

    fn take_outcome(&self) -> Outcome {
        self.outcome
            .lock()
            .unwrap()
            .take()
            .expect("no outcome recorded: check_narration didn't call the agent")
    }
}

impl Agent for MockAgent {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn parse_hook_input(&self, _hook_type: HookType) -> HookInput {
        std::mem::take(&mut *self.input.lock().unwrap())
    }

    fn session_start(&self, _input: &HookInput, _is_listening: bool) -> anyhow::Result<()> {
        unimplemented!("not used by check_narration")
    }

    fn editor_context(&self, _state: &EditorState) -> anyhow::Result<()> {
        unimplemented!("not used by check_narration")
    }

    fn attend_activate(&self, _session_id: &SessionId) -> anyhow::Result<()> {
        *self.outcome.lock().unwrap() = Some(Outcome::Activation);
        Ok(())
    }

    fn attend_deactivate(&self, _session_id: &SessionId) -> anyhow::Result<()> {
        unimplemented!("not used by check_narration")
    }

    fn deliver_narration(&self, content: &str) -> anyhow::Result<()> {
        *self.outcome.lock().unwrap() = Some(Outcome::Narration(content.to_string()));
        Ok(())
    }

    fn attend_result(&self, decision: &HookDecision, _hook_type: HookType) -> anyhow::Result<()> {
        *self.outcome.lock().unwrap() = Some(Outcome::Decision(decision.clone()));
        Ok(())
    }

    fn install(&self, _bin_cmd: &str, _project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        unimplemented!("not used by check_narration")
    }

    fn uninstall(&self, _project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        unimplemented!("not used by check_narration")
    }
}

/// Which `attend listen` variant to simulate.
#[derive(Clone, Copy)]
pub(super) enum ListenVariant {
    /// Not an `attend listen` command.
    None,
    /// `attend listen` (start/wait).
    Listen,
    /// `attend listen --stop` (deactivation).
    ListenStop,
}

/// Test harness that redirects all state I/O to a temp directory.
///
/// Wraps [`state::CacheDirGuard`] and adds hook-specific helpers
/// (activate, write_pending, fire_hook, etc.).
pub(super) struct TestHarness {
    guard: state::CacheDirGuard,
}

impl TestHarness {
    pub(super) fn new() -> Self {
        Self {
            guard: state::CacheDirGuard::new(),
        }
    }

    fn cache(&self) -> &Utf8PathBuf {
        &self.guard.cache
    }

    /// Simulate `/attend` activation: write the listening file and the
    /// activated marker, just like `user_prompt` does for `/attend`.
    pub(super) fn activate(&self, session_id: &SessionId) {
        // Write listening file
        let listening = self.cache().join("listening");
        std::fs::write(&listening, session_id.as_str()).unwrap();
        // Write activated marker
        let marker = self
            .cache()
            .join("sessions/activated")
            .join(session_id.as_str());
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "").unwrap();
        // Clear any stale moved marker (like user_prompt does)
        let moved = self
            .cache()
            .join("sessions/moved")
            .join(session_id.as_str());
        let _ = std::fs::remove_file(&moved);
    }

    /// Write a pending narration file for the given session.
    ///
    /// Creates a minimal Words event so the delivery path has content
    /// to render. Uses an atomic counter for unique, ordered filenames
    /// (safe for rapid proptest sequences without sleeping).
    pub(super) fn write_pending(&self, session_id: &SessionId, text: &str) {
        let dir = self.cache().join("pending").join(session_id.as_str());
        std::fs::create_dir_all(&dir).unwrap();
        let seq = PENDING_SEQ.fetch_add(1, Ordering::Relaxed);
        let filename = format!("{seq:020}.json");
        let events = vec![Event::Words {
            timestamp: ts(0.0),
            text: text.to_string(),
        }];
        let content = serde_json::to_string(&events).unwrap();
        std::fs::write(dir.join(filename), content).unwrap();
    }

    /// Write a pending narration file whose content will be filtered out
    /// during delivery (path outside the test cwd).
    ///
    /// The file exists on disk (so `collect_pending` finds it), but
    /// `read_pending` returns `None` because the event's path doesn't
    /// match the session's working directory.
    pub(super) fn write_undeliverable_pending(&self, session_id: &SessionId) {
        let dir = self.cache().join("pending").join(session_id.as_str());
        std::fs::create_dir_all(&dir).unwrap();
        let seq = PENDING_SEQ.fetch_add(1, Ordering::Relaxed);
        let filename = format!("{seq:020}.json");
        // FileDiff with a path outside any test cwd. read_pending filters
        // by cwd, so this event will be dropped, yielding None.
        let events = vec![Event::FileDiff {
            timestamp: ts(0.0),
            path: "/nonexistent/outside/project/foo.rs".to_string(),
            old: "old".to_string(),
            new: "new".to_string(),
        }];
        let content = serde_json::to_string(&events).unwrap();
        std::fs::write(dir.join(filename), content).unwrap();
    }

    /// Simulate a running receiver by writing a lock file with our PID.
    pub(super) fn fake_receiver(&self) -> ReceiverGuard {
        let lock_path = self.cache().join("receive.lock");
        std::fs::write(&lock_path, std::process::id().to_string()).unwrap();
        ReceiverGuard { lock_path }
    }

    /// Fire a hook and return what the agent was told.
    pub(super) fn fire_hook(
        &self,
        session_id: &SessionId,
        hook_type: HookType,
        is_listen: bool,
        stop_hook_active: bool,
    ) -> Outcome {
        let variant = if is_listen {
            ListenVariant::Listen
        } else {
            ListenVariant::None
        };
        self.fire_hook_ext(session_id, hook_type, variant, stop_hook_active)
    }

    /// Fire a hook with explicit listen variant control.
    pub(super) fn fire_hook_ext(
        &self,
        session_id: &SessionId,
        hook_type: HookType,
        listen_variant: ListenVariant,
        stop_hook_active: bool,
    ) -> Outcome {
        let kind = match hook_type {
            HookType::Stop => HookKind::Stop { stop_hook_active },
            HookType::PreToolUse | HookType::PostToolUse => match listen_variant {
                ListenVariant::Listen => HookKind::ToolUse {
                    bash_command: Some(listen_command()),
                },
                ListenVariant::ListenStop => HookKind::ToolUse {
                    bash_command: Some(listen_stop_command()),
                },
                ListenVariant::None => HookKind::ToolUse {
                    bash_command: Some("some-other-tool".to_string()),
                },
            },
            _ => HookKind::default(),
        };

        let input = HookInput {
            session_id: Some(session_id.clone()),
            cwd: Some(self.cache().clone()),
            kind,
        };

        let agent = MockAgent::new(input);
        check_narration(&agent, hook_type).expect("check_narration failed");
        agent.take_outcome()
    }

    /// Assert the outcome is a specific decision.
    pub(super) fn assert_decision(outcome: &Outcome, expected: &HookDecision) {
        match outcome {
            Outcome::Decision(d) => assert_eq!(d, expected, "expected {expected:?}, got {d:?}"),
            Outcome::Narration(c) => {
                panic!("expected decision {expected:?}, got narration delivery: {c}")
            }
            Outcome::Activation => {
                panic!("expected decision {expected:?}, got activation")
            }
        }
    }

    /// Assert the outcome is an activation (auto-claim).
    pub(super) fn assert_activation(outcome: &Outcome) {
        match outcome {
            Outcome::Activation => {}
            other => panic!("expected activation, got {other:?}"),
        }
    }

    /// Assert the outcome is narration delivery containing the given text.
    pub(super) fn assert_narration(outcome: &Outcome, expected_substring: &str) {
        match outcome {
            Outcome::Narration(content) => assert!(
                content.contains(expected_substring),
                "narration should contain {expected_substring:?}, got: {content}"
            ),
            Outcome::Decision(d) => {
                panic!("expected narration containing {expected_substring:?}, got decision: {d:?}")
            }
            Outcome::Activation => {
                panic!("expected narration containing {expected_substring:?}, got activation")
            }
        }
    }
}

/// RAII guard that removes the fake receiver lock on drop.
pub(super) struct ReceiverGuard {
    lock_path: Utf8PathBuf,
}

impl Drop for ReceiverGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Build a bash command string that `detect_listen_command` will recognize,
/// matching against the current test binary's filename.
fn listen_command() -> String {
    let exe = std::env::current_exe().expect("can't determine test binary path");
    format!("{} listen", exe.display())
}

/// Build a bash command string for `attend listen --stop`.
fn listen_stop_command() -> String {
    let exe = std::env::current_exe().expect("can't determine test binary path");
    format!("{} listen --stop", exe.display())
}
