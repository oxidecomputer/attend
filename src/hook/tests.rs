use super::*;
use decision::SessionRelation;

// ---------------------------------------------------------------------------
// Exhaustive enumeration of the decision space
//
// general_decision has 5 inputs: relation (3) × has_pending (2) ×
// receiver_alive (2) × stop_hook_active (2) × hook_type (3) = 72
// combinations. Small enough to enumerate exhaustively, giving complete
// coverage with no randomness.
// ---------------------------------------------------------------------------

const ALL_RELATIONS: [SessionRelation; 3] = [
    SessionRelation::Active,
    SessionRelation::Stolen,
    SessionRelation::Inactive,
];

/// Only the hook types that reach `general_decision`. SessionStart and
/// UserPrompt are handled by separate code paths.
const ALL_HOOK_TYPES: [HookType; 3] = [HookType::Stop, HookType::PreToolUse, HookType::PostToolUse];

const ALL_BOOLS: [bool; 2] = [false, true];

/// Invoke `f` for every one of the 72 input combinations.
fn for_all(mut f: impl FnMut(SessionRelation, bool, bool, bool, HookType)) {
    for &relation in &ALL_RELATIONS {
        for &has_pending in &ALL_BOOLS {
            for &receiver_alive in &ALL_BOOLS {
                for &stop_hook_active in &ALL_BOOLS {
                    for &hook_type in &ALL_HOOK_TYPES {
                        f(
                            relation,
                            has_pending,
                            receiver_alive,
                            stop_hook_active,
                            hook_type,
                        );
                    }
                }
            }
        }
    }
}

/// Decision helpers for readable assertions.
fn is_block(d: &HookDecision) -> bool {
    matches!(
        d,
        HookDecision::Guidance {
            effect: GuidanceEffect::Block,
            ..
        }
    )
}

fn effect_of(d: &HookDecision) -> Option<GuidanceEffect> {
    match d {
        HookDecision::Silent => None,
        HookDecision::Guidance { effect, .. } => Some(*effect),
    }
}

fn reason_of(d: &HookDecision) -> Option<&GuidanceReason> {
    match d {
        HookDecision::Silent => None,
        HookDecision::Guidance { reason, .. } => Some(reason),
    }
}

// ---------------------------------------------------------------------------
// Invariant tests — each checks one property across all 72 combinations
//
// Documented with:
//  - the invariant being verified
//  - the concurrent flow / failure mode it guards against
// ---------------------------------------------------------------------------

/// **Invariant**: Inactive and Stolen sessions never receive a Block.
///
/// **Flow**: Multiple Claude Code sessions are open. Some have never run
/// `/attend` (Inactive). Others had narration stolen away (Stolen).
/// Neither should have tool calls or stop hooks blocked — that would
/// disrupt unrelated work in sessions that aren't participating in
/// (or have been displaced from) narration.
#[test]
fn non_active_sessions_never_blocked() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            if relation == SessionRelation::Active {
                return;
            }
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            assert!(
                !is_block(&d),
                "non-active session got Block: relation={relation:?}, \
             has_pending={has_pending}, receiver_alive={receiver_alive}, \
             stop_hook_active={stop_hook_active}, hook_type={hook_type:?}, \
             decision={d:?}"
            );
        },
    );
}

/// **Invariant**: SessionMoved is never delivered as a Block.
///
/// **Flow**: Session A was the active listener. Session B runs `/attend`,
/// stealing narration. Session A's next hook fires. The SessionMoved
/// notification tells A "narration moved away" but must not block A's
/// tools — A might be mid-task, writing files, running tests. Blocking
/// would disrupt that work for no benefit.
#[test]
fn session_moved_is_never_block() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            if reason_of(&d) == Some(&GuidanceReason::SessionMoved) {
                assert_eq!(
                    effect_of(&d),
                    Some(GuidanceEffect::Approve),
                    "SessionMoved should be Approve: relation={relation:?}, hook_type={hook_type:?}"
                );
            }
        },
    );
}

/// **Invariant**: Inactive sessions always produce Silent — no output at
/// all, regardless of other state.
///
/// **Flow**: A session that never activated `/attend` fires a hook while
/// some other session is listening, or while no session is listening, or
/// while a receiver is alive somewhere. None of that matters: this
/// session is not a narration participant and should be completely
/// unaware of the attend system.
#[test]
fn inactive_always_silent() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            if relation != SessionRelation::Inactive {
                return;
            }
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::Silent,
                "Inactive should always be Silent: has_pending={has_pending}, \
             receiver_alive={receiver_alive}, hook_type={hook_type:?}"
            );
        },
    );
}

/// **Invariant**: A stolen session's decision is independent of
/// `has_pending`, `receiver_alive`, `stop_hook_active`, and `hook_type`.
///
/// **Flow**: Session A was displaced. Meanwhile narration is piling up,
/// the receiver crashed, or the stop hook is re-firing. None of that
/// is A's concern anymore — it has been displaced. The decision should
/// be a fixed advisory regardless of what's happening in the narration
/// subsystem. Checking narration state for a stolen session would be
/// a bug: it could cause a displaced session to start a receiver or
/// attempt delivery for content it no longer owns.
#[test]
fn stolen_decision_ignores_other_state() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            if relation != SessionRelation::Stolen {
                return;
            }
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::approve(GuidanceReason::SessionMoved),
                "Stolen should always be Approve(SessionMoved): has_pending={has_pending}, \
             receiver_alive={receiver_alive}, hook_type={hook_type:?}"
            );
        },
    );
}

/// **Invariant**: When the active session has pending narration, the
/// decision is Block(NarrationReady) regardless of receiver state,
/// stop_hook_active, or hook type.
///
/// **Flow**: The user is narrating, and events have been written to the
/// pending directory. The agent is mid-response, making tool calls. If
/// there's pending narration, we *must* block: this forces the agent to
/// run `attend listen` to pick up the content before continuing. If we
/// didn't block, narration could go stale indefinitely while the agent
/// keeps working.
///
/// The "regardless of receiver state" part matters because of a race:
/// the receiver might be technically alive but hasn't consumed the
/// pending files yet. Or the receiver might have crashed right after
/// files appeared. Either way, pending files = block.
#[test]
fn pending_narration_always_blocks() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            if relation != SessionRelation::Active || !has_pending {
                return;
            }
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::block(GuidanceReason::NarrationReady),
                "Active + pending should always Block(NarrationReady): \
             receiver_alive={receiver_alive}, stop_hook_active={stop_hook_active}, \
             hook_type={hook_type:?}"
            );
        },
    );
}

/// **Invariant**: When the active session has no pending narration, no
/// receiver, and this is the first attempt (not a re-invocation), the
/// decision always carries `StartReceiver`.
///
/// **Flow**: The receiver process crashed (or was never started). The
/// agent doesn't know yet. Without a receiver, future narration will
/// pile up in the pending directory with no delivery mechanism. The hook
/// must notice the gap and tell the agent to start one. If it doesn't,
/// the user's narration goes undelivered until something else triggers
/// receiver startup.
#[test]
fn missing_receiver_detected() {
    for &hook_type in &ALL_HOOK_TYPES {
        let d = general_decision(
            SessionRelation::Active,
            false, // no pending
            false, // no receiver
            false, // first attempt
            hook_type,
        );
        assert_eq!(
            reason_of(&d),
            Some(&GuidanceReason::StartReceiver),
            "missing receiver should produce StartReceiver: hook_type={hook_type:?}"
        );
    }
}

/// **Invariant**: When stop_hook_active is true and there's no pending
/// narration, the decision is Silent regardless of receiver state.
///
/// **Flow**: The Stop hook fired, returned Block(StartReceiver), and
/// Claude Code re-invoked the hook with stop_hook_active=true. If we
/// block again, we get an infinite loop: block -> re-invoke -> block ->
/// re-invoke. The safety valve MUST release. This is especially
/// important in the race where the agent started the receiver but its
/// lock file hasn't been written yet: receiver_alive is false but
/// blocking again would be wrong.
///
/// The only exception is pending narration (tested separately in
/// `pending_narration_always_blocks`): if narration arrived during
/// the re-invocation cycle, we block with NarrationReady, not
/// StartReceiver. That's safe because NarrationReady is a different
/// action (run `attend listen`) that breaks the StartReceiver loop.
#[test]
fn reentry_safety_valve_releases() {
    for &receiver_alive in &ALL_BOOLS {
        for &hook_type in &ALL_HOOK_TYPES {
            let d = general_decision(
                SessionRelation::Active,
                false, // no pending
                receiver_alive,
                true, // stop_hook_active: re-invocation
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::Silent,
                "re-invocation should release to Silent: receiver_alive={receiver_alive}, \
                 hook_type={hook_type:?}"
            );
        }
    }
}

/// **Invariant**: StartReceiver uses Block on Stop hooks but Approve on
/// ToolUse hooks.
///
/// **Flow**: The receiver is dead and needs restarting. Two scenarios:
///
/// 1. **Stop hook**: The agent is about to exit. If we approve, the
///    session exits with no receiver, and future narration goes
///    undelivered. We MUST block to give the agent a chance to start
///    the receiver before exiting.
///
/// 2. **PreToolUse/PostToolUse**: The agent is executing a tool (e.g.
///    reading a file, running a test). Blocking that tool call just
///    because the receiver is down is too disruptive. Instead, approve
///    the tool but inject an advisory nudge.
#[test]
fn start_receiver_effect_by_hook_type() {
    for &hook_type in &ALL_HOOK_TYPES {
        let d = general_decision(
            SessionRelation::Active,
            false, // no pending
            false, // no receiver
            false, // first attempt
            hook_type,
        );
        let expected = match hook_type {
            HookType::Stop => GuidanceEffect::Block,
            _ => GuidanceEffect::Approve,
        };
        assert_eq!(
            effect_of(&d),
            Some(expected),
            "StartReceiver effect wrong for hook_type={hook_type:?}"
        );
    }
}

/// **Invariant**: NarrationReady is always a Block, regardless of hook
/// type.
///
/// **Flow**: Narration has arrived and is sitting in pending files.
/// Whether this is a Stop hook (agent exiting), PreToolUse (about to
/// run a tool), or PostToolUse (tool just ran), the agent must pick up
/// the narration before continuing. This is the synchronous delivery
/// trigger: the agent runs `attend listen`, its PreToolUse hook delivers
/// the content and starts a new receiver in one round trip.
///
/// If we approved instead of blocking, the agent would continue without
/// the narration content, and the user's spoken context would go
/// undelivered.
#[test]
fn narration_ready_always_blocks() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            if reason_of(&d) == Some(&GuidanceReason::NarrationReady) {
                assert!(
                    is_block(&d),
                    "NarrationReady should always block: relation={relation:?}, \
                 hook_type={hook_type:?}, decision={d:?}"
                );
            }
        },
    );
}

/// **Invariant**: When the active session has no pending narration and a
/// receiver is alive, the decision is Silent regardless of other flags.
///
/// **Flow**: Everything is working normally. The receiver is running in
/// the background, polling for new narration events. No events have
/// arrived yet (no pending). The hook should be completely transparent:
/// the receiver will handle delivery when narration arrives. Any
/// non-Silent output here would be noise.
#[test]
fn receiver_alive_no_pending_is_silent() {
    for &stop_hook_active in &ALL_BOOLS {
        for &hook_type in &ALL_HOOK_TYPES {
            let d = general_decision(
                SessionRelation::Active,
                false, // no pending
                true,  // receiver alive
                stop_hook_active,
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::Silent,
                "receiver alive + no pending should be Silent: \
                 stop_hook_active={stop_hook_active}, hook_type={hook_type:?}"
            );
        }
    }
}

// --- general_decision point tests ---

/// Inactive session (no listening session or no session ID): silent.
#[test]
fn general_inactive_silent() {
    let d = general_decision(
        SessionRelation::Inactive,
        false,
        false,
        false,
        HookType::Stop,
    );
    assert_eq!(d, HookDecision::Silent);
}

/// Stolen session: advisory SessionMoved (approve, not block).
#[test]
fn general_stolen_session_moved() {
    let d = general_decision(SessionRelation::Stolen, false, false, false, HookType::Stop);
    assert_eq!(d, HookDecision::approve(GuidanceReason::SessionMoved));
}

/// Active session with pending narration: block with NarrationReady.
#[test]
fn general_active_pending_narration() {
    let d = general_decision(SessionRelation::Active, true, false, false, HookType::Stop);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Pending narration takes priority over a running receiver.
#[test]
fn general_pending_takes_priority_over_receiver() {
    let d = general_decision(SessionRelation::Active, true, true, false, HookType::Stop);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Pending narration takes priority even on re-invocation.
#[test]
fn general_pending_takes_priority_over_reentry() {
    let d = general_decision(SessionRelation::Active, true, false, true, HookType::Stop);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Receiver alive, no pending: silent.
#[test]
fn general_active_receiver_alive_no_pending() {
    let d = general_decision(SessionRelation::Active, false, true, false, HookType::Stop);
    assert_eq!(d, HookDecision::Silent);
}

/// No receiver, no pending, first attempt on Stop: block to start receiver.
#[test]
fn general_stop_no_receiver_blocks() {
    let d = general_decision(SessionRelation::Active, false, false, false, HookType::Stop);
    assert_eq!(d, HookDecision::block(GuidanceReason::StartReceiver));
}

/// No receiver, no pending, first attempt on PreToolUse: advisory to start receiver.
#[test]
fn general_pre_tool_use_no_receiver_approves() {
    let d = general_decision(
        SessionRelation::Active,
        false,
        false,
        false,
        HookType::PreToolUse,
    );
    assert_eq!(d, HookDecision::approve(GuidanceReason::StartReceiver));
}

/// Re-invocation after a previous block, no receiver: silent to avoid loop.
#[test]
fn general_active_reentry_no_receiver_silent() {
    let d = general_decision(SessionRelation::Active, false, false, true, HookType::Stop);
    assert_eq!(d, HookDecision::Silent);
}

/// Re-invocation with receiver alive: silent.
#[test]
fn general_active_reentry_receiver_alive_silent() {
    let d = general_decision(SessionRelation::Active, false, true, true, HookType::Stop);
    assert_eq!(d, HookDecision::Silent);
}

// --- is_attend_prompt tests ---

/// Exact `/attend` match.
#[test]
fn is_attend_prompt_exact() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("/attend".into()),
        },
        ..Default::default()
    };
    assert!(is_attend_prompt(&input));
}

/// `/attend` with surrounding whitespace.
#[test]
fn is_attend_prompt_with_whitespace() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("  /attend  ".into()),
        },
        ..Default::default()
    };
    assert!(is_attend_prompt(&input));
}

/// Non-attend prompt text.
#[test]
fn is_attend_prompt_different_text() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("hello world".into()),
        },
        ..Default::default()
    };
    assert!(!is_attend_prompt(&input));
}

/// No prompt field at all.
#[test]
fn is_attend_prompt_no_prompt_field() {
    let input = HookInput::default();
    assert!(!is_attend_prompt(&input));
}

/// Partial match: `/attend to this` should not match.
#[test]
fn is_attend_prompt_partial() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("/attend to this".into()),
        },
        ..Default::default()
    };
    assert!(!is_attend_prompt(&input));
}

// --- is_listen_command tests ---

/// Bare binary name matches.
#[test]
fn listen_command_bare_name() {
    assert!(is_listen_command("attend listen", "attend"));
}

/// Full path matches against filename component.
#[test]
fn listen_command_full_path() {
    assert!(is_listen_command("/usr/local/bin/attend listen", "attend"));
}

/// Extra flags after `listen` are allowed.
#[test]
fn listen_command_with_flags() {
    assert!(is_listen_command("attend listen --check", "attend"));
}

/// Different subcommand is not matched.
#[test]
fn listen_command_different_subcommand() {
    assert!(!is_listen_command("attend narrate status", "attend"));
}

/// Different binary name is not matched.
#[test]
fn listen_command_different_binary() {
    assert!(!is_listen_command("cargo test", "attend"));
}

/// Empty command is not matched.
#[test]
fn listen_command_empty() {
    assert!(!is_listen_command("", "attend"));
}

/// Binary-only (no subcommand) is not matched.
#[test]
fn listen_command_no_subcommand() {
    assert!(!is_listen_command("attend", "attend"));
}

// ---------------------------------------------------------------------------
// Stateful (model-based) integration tests
//
// These tests exercise check_narration against real filesystem state in a
// temp directory. A TestHarness redirects cache_dir() via the thread-local
// override in state.rs, then manipulates files (listening marker, activated
// markers, pending narration, receive lock) to simulate multi-session
// scenarios. A MockAgent records the output calls so tests can assert on
// the actual decisions made by the full check_narration code path.
// ---------------------------------------------------------------------------

use std::sync::Mutex;

use camino::Utf8PathBuf;
use tempfile::TempDir;

use crate::agent::Agent;
use crate::narrate::merge::Event;
use crate::state::{self, EditorState, SessionId};

/// What check_narration communicated back to the agent.
#[derive(Debug)]
enum Outcome {
    /// `agent.attend_result` was called.
    Decision(HookDecision),
    /// `agent.deliver_narration` was called with this content.
    Narration(String),
}

/// Mock agent that records hook output for assertion.
///
/// The `input` field is set by the harness before each call so
/// `parse_hook_input` returns the right session/tool context.
struct MockAgent {
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
        unimplemented!("not used by check_narration")
    }

    fn deliver_narration(&self, content: &str) -> anyhow::Result<()> {
        *self.outcome.lock().unwrap() = Some(Outcome::Narration(content.to_string()));
        Ok(())
    }

    fn attend_result(&self, decision: &HookDecision, _hook_type: HookType) -> anyhow::Result<()> {
        // Reconstruct the decision (HookDecision doesn't derive Clone).
        let cloned = match decision {
            HookDecision::Silent => HookDecision::Silent,
            HookDecision::Guidance { reason, effect } => HookDecision::Guidance {
                reason: reason.clone(),
                effect: *effect,
            },
        };
        *self.outcome.lock().unwrap() = Some(Outcome::Decision(cloned));
        Ok(())
    }

    fn install(&self, _bin_cmd: &str, _project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        unimplemented!("not used by check_narration")
    }

    fn uninstall(&self, _project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        unimplemented!("not used by check_narration")
    }
}

/// Test harness that redirects all state I/O to a temp directory.
///
/// On creation, sets the thread-local cache_dir override. On drop,
/// clears it. Each test gets an isolated filesystem namespace.
struct TestHarness {
    _tmp: TempDir,
    cache: Utf8PathBuf,
}

impl TestHarness {
    fn new() -> Self {
        let tmp = TempDir::new().expect("failed to create temp dir");
        let cache = Utf8PathBuf::try_from(tmp.path().to_path_buf()).expect("non-UTF-8 temp dir");
        state::set_cache_dir_override(Some(cache.clone()));
        std::fs::create_dir_all(&cache).expect("failed to create cache dir");
        Self { _tmp: tmp, cache }
    }

    /// Simulate `/attend` activation: write the listening file and the
    /// activated marker, just like `user_prompt` does for `/attend`.
    fn activate(&self, session_id: &SessionId) {
        // Write listening file
        let listening = self.cache.join("listening");
        std::fs::write(&listening, session_id.as_str()).unwrap();
        // Write activated marker
        let marker = self.cache.join(format!("activated-{session_id}"));
        std::fs::write(&marker, "").unwrap();
        // Clear any stale moved marker (like user_prompt does)
        let moved = self.cache.join(format!("moved-{session_id}"));
        let _ = std::fs::remove_file(&moved);
    }

    /// Write a pending narration file for the given session.
    ///
    /// Creates a minimal Words event so the delivery path has content
    /// to render. Returns the path of the created file.
    fn write_pending(&self, session_id: &SessionId, text: &str) {
        let dir = self.cache.join("pending").join(session_id.as_str());
        std::fs::create_dir_all(&dir).unwrap();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let filename = format!("{}.json", ts.as_nanos());
        let events = vec![Event::Words {
            offset_secs: 0.0,
            text: text.to_string(),
        }];
        let content = serde_json::to_string(&events).unwrap();
        std::fs::write(dir.join(filename), content).unwrap();
    }

    /// Simulate a running receiver by writing a lock file with our PID.
    fn fake_receiver(&self) -> ReceiverGuard {
        let lock_path = self.cache.join("receive.lock");
        std::fs::write(&lock_path, std::process::id().to_string()).unwrap();
        ReceiverGuard { lock_path }
    }

    /// Fire a hook and return what the agent was told.
    fn fire_hook(
        &self,
        session_id: &SessionId,
        hook_type: HookType,
        is_listen: bool,
        stop_hook_active: bool,
    ) -> Outcome {
        let kind = match hook_type {
            HookType::Stop => HookKind::Stop { stop_hook_active },
            HookType::PreToolUse | HookType::PostToolUse => {
                if is_listen {
                    HookKind::ToolUse {
                        bash_command: Some(listen_command()),
                    }
                } else {
                    HookKind::ToolUse {
                        bash_command: Some("some-other-tool".to_string()),
                    }
                }
            }
            _ => HookKind::default(),
        };

        let input = HookInput {
            session_id: Some(session_id.clone()),
            cwd: Some(self.cache.clone()),
            kind,
        };

        let agent = MockAgent::new(input);
        check_narration(&agent, hook_type).expect("check_narration failed");
        agent.take_outcome()
    }

    /// Assert the outcome is a specific decision.
    fn assert_decision(outcome: &Outcome, expected: &HookDecision) {
        match outcome {
            Outcome::Decision(d) => assert_eq!(d, expected, "expected {expected:?}, got {d:?}"),
            Outcome::Narration(c) => {
                panic!("expected decision {expected:?}, got narration delivery: {c}")
            }
        }
    }

    /// Assert the outcome is narration delivery containing the given text.
    fn assert_narration(outcome: &Outcome, expected_substring: &str) {
        match outcome {
            Outcome::Narration(content) => assert!(
                content.contains(expected_substring),
                "narration should contain {expected_substring:?}, got: {content}"
            ),
            Outcome::Decision(d) => {
                panic!("expected narration containing {expected_substring:?}, got decision: {d:?}")
            }
        }
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        state::set_cache_dir_override(None);
    }
}

/// RAII guard that removes the fake receiver lock on drop.
struct ReceiverGuard {
    lock_path: Utf8PathBuf,
}

impl Drop for ReceiverGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Build a bash command string that `is_attend_listen` will recognize,
/// matching against the current test binary's filename.
fn listen_command() -> String {
    let exe = std::env::current_exe().expect("can't determine test binary path");
    format!("{} listen", exe.display())
}

// ---------------------------------------------------------------------------
// Scenario tests
// ---------------------------------------------------------------------------

/// **Scenario**: A session that never activated `/attend` should be
/// completely invisible to the hook system.
///
/// **Flow**: An unrelated Claude Code session fires hooks while another
/// session is actively listening. The non-participant should never see
/// any narration-related output.
#[test]
fn non_participant_is_invisible() {
    let h = TestHarness::new();
    let active: SessionId = "active".into();
    let bystander: SessionId = "bystander".into();

    h.activate(&active);

    // Bystander fires every hook type — all should be silent.
    for &ht in &ALL_HOOK_TYPES {
        let out = h.fire_hook(&bystander, ht, false, false);
        TestHarness::assert_decision(&out, &HookDecision::Silent);
    }

    // Even with pending narration for the active session, bystander is silent.
    h.write_pending(&active, "hello world");
    for &ht in &ALL_HOOK_TYPES {
        let out = h.fire_hook(&bystander, ht, false, false);
        TestHarness::assert_decision(&out, &HookDecision::Silent);
    }
}

/// **Scenario**: Full lifecycle of a single session from activation through
/// narration delivery.
///
/// **Flow**: Session activates → first hook nudges to start receiver →
/// receiver starts → hooks go silent → narration arrives → hook blocks
/// with NarrationReady → `attend listen` delivers content → hooks go
/// silent again.
#[test]
fn single_session_lifecycle() {
    let h = TestHarness::new();
    let s: SessionId = "session-1".into();

    // Before activation: silent.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);

    // Activate.
    h.activate(&s);

    // No receiver yet: nudge to start one.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::StartReceiver));

    // Stop hook with no receiver: block (don't exit without a receiver).
    let out = h.fire_hook(&s, HookType::Stop, false, false);
    TestHarness::assert_decision(&out, &HookDecision::block(GuidanceReason::StartReceiver));

    // Start receiver.
    let _receiver = h.fake_receiver();

    // With receiver running: silent.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);

    let out = h.fire_hook(&s, HookType::Stop, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);

    // Narration arrives.
    h.write_pending(&s, "look at this function");

    // General hook: block with NarrationReady.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::block(GuidanceReason::NarrationReady));

    // attend listen PreToolUse: delivers narration content.
    let out = h.fire_hook(&s, HookType::PreToolUse, true, false);
    TestHarness::assert_narration(&out, "look at this function");

    // After delivery, pending is archived — hooks go silent again.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);
}

/// **Scenario**: Session stealing — session B takes over narration from A.
///
/// **Flow**: A activates → B activates (steals) → A's hooks get advisory
/// SessionMoved (once) then go silent → B's hooks behave as the active
/// session → A's `attend listen` is blocked (anti-livelock).
#[test]
fn session_stealing() {
    let h = TestHarness::new();
    let a: SessionId = "session-a".into();
    let b: SessionId = "session-b".into();

    // A activates.
    h.activate(&a);
    let _receiver = h.fake_receiver();
    let out = h.fire_hook(&a, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);

    // B steals narration.
    h.activate(&b);

    // A's next general hook: advisory SessionMoved.
    let out = h.fire_hook(&a, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::SessionMoved));

    // A's second general hook: ratchet suppresses — silent.
    let out = h.fire_hook(&a, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);

    // A tries `attend listen`: blocked (anti-livelock).
    let out = h.fire_hook(&a, HookType::PreToolUse, true, false);
    TestHarness::assert_decision(&out, &HookDecision::block(GuidanceReason::SessionMoved));

    // B is now the active session — needs a receiver.
    // (The old receiver from A's time is still "alive" in the lock file.)
    let out = h.fire_hook(&b, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);
}

/// **Scenario**: The SessionMoved ratchet resets when a session re-activates
/// with `/attend`.
///
/// **Flow**: A activates → B steals → A gets SessionMoved (once) → A
/// re-activates with /attend → B steals again → A gets SessionMoved
/// again (ratchet was reset).
#[test]
fn moved_ratchet_resets_on_reactivation() {
    let h = TestHarness::new();
    let a: SessionId = "session-a".into();
    let b: SessionId = "session-b".into();

    // A activates, B steals.
    h.activate(&a);
    h.activate(&b);

    // A gets SessionMoved once.
    let out = h.fire_hook(&a, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::SessionMoved));

    // Ratchet: second time is silent.
    let out = h.fire_hook(&a, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);

    // A re-activates (steals back).
    h.activate(&a);

    // B steals again.
    h.activate(&b);

    // A should get SessionMoved again — ratchet was reset by re-activation.
    let out = h.fire_hook(&a, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::SessionMoved));
}

/// **Scenario**: The stop_hook_active safety valve prevents infinite
/// block loops on the Stop hook.
///
/// **Flow**: Active session, no receiver, Stop fires → Block(StartReceiver).
/// Claude Code re-invokes with stop_hook_active=true → Silent (safety valve).
/// This prevents: block → re-invoke → block → re-invoke → ...
#[test]
fn stop_reentry_safety_valve() {
    let h = TestHarness::new();
    let s: SessionId = "session-1".into();
    h.activate(&s);

    // First Stop: blocks to start receiver.
    let out = h.fire_hook(&s, HookType::Stop, false, false);
    TestHarness::assert_decision(&out, &HookDecision::block(GuidanceReason::StartReceiver));

    // Re-invocation: safety valve releases.
    let out = h.fire_hook(&s, HookType::Stop, false, true);
    TestHarness::assert_decision(&out, &HookDecision::Silent);
}

/// **Scenario**: `attend listen` PostToolUse always gets the
/// ListenerStarted advisory, regardless of session state.
///
/// **Flow**: The command already ran (PostToolUse). The advisory primes
/// the agent to restart (not read output from) the listener when the
/// task notification arrives.
#[test]
fn listen_post_tool_use_always_advisory() {
    let h = TestHarness::new();
    let s: SessionId = "session-1".into();
    h.activate(&s);

    let out = h.fire_hook(&s, HookType::PostToolUse, true, false);
    TestHarness::assert_decision(
        &out,
        &HookDecision::approve(GuidanceReason::ListenerStarted),
    );
}

/// **Scenario**: `attend listen` is blocked when a receiver is already
/// running, to prevent duplicate listeners.
///
/// **Flow**: Session is active, receiver is alive. Agent tries to start
/// another listener via `attend listen`. PreToolUse blocks with
/// ListenerAlreadyActive.
#[test]
fn listen_blocked_when_receiver_alive() {
    let h = TestHarness::new();
    let s: SessionId = "session-1".into();
    h.activate(&s);
    let _receiver = h.fake_receiver();

    let out = h.fire_hook(&s, HookType::PreToolUse, true, false);
    TestHarness::assert_decision(
        &out,
        &HookDecision::block(GuidanceReason::ListenerAlreadyActive),
    );
}

/// **Scenario**: `attend listen` on an active session with no receiver
/// and no pending narration is allowed silently — this is the normal
/// startup path.
#[test]
fn listen_allowed_when_no_receiver_no_pending() {
    let h = TestHarness::new();
    let s: SessionId = "session-1".into();
    h.activate(&s);

    let out = h.fire_hook(&s, HookType::PreToolUse, true, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);
}

/// **Scenario**: Multiple narrations accumulate and are delivered together
/// in a single `attend listen` round trip.
///
/// **Flow**: Two narrations arrive before the agent picks them up.
/// The PreToolUse hook on `attend listen` delivers both, archives them,
/// and subsequent hooks see no pending content.
#[test]
fn batched_narration_delivery() {
    let h = TestHarness::new();
    let s: SessionId = "session-1".into();
    h.activate(&s);

    // Two narrations arrive.
    h.write_pending(&s, "first narration");
    // Small delay so filenames differ (nanosecond timestamps).
    std::thread::sleep(std::time::Duration::from_millis(1));
    h.write_pending(&s, "second narration");

    // attend listen delivers both.
    let out = h.fire_hook(&s, HookType::PreToolUse, true, false);
    TestHarness::assert_narration(&out, "first narration");
    TestHarness::assert_narration(&out, "second narration");

    // After delivery, no more pending.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::StartReceiver));
}

/// **Scenario**: Narration for session A is not visible to session B,
/// even if B is the active listener.
///
/// **Flow**: A activates, narration arrives for A. B steals. B's hooks
/// don't see A's pending narration (pending is per-session). A's hooks
/// report SessionMoved, not NarrationReady.
#[test]
fn pending_narration_is_per_session() {
    let h = TestHarness::new();
    let a: SessionId = "session-a".into();
    let b: SessionId = "session-b".into();

    h.activate(&a);
    h.write_pending(&a, "narration for A");

    // B steals.
    h.activate(&b);

    // B has no pending narration — gets StartReceiver (no receiver).
    let out = h.fire_hook(&b, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::StartReceiver));

    // A is displaced — gets SessionMoved, not NarrationReady.
    let out = h.fire_hook(&a, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::SessionMoved));
}
