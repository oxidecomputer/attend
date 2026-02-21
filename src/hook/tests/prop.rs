use proptest::prelude::*;

use super::super::*;
use super::harness::{ReceiverGuard, TestHarness};
use crate::state::SessionId;
use decision::SessionRelation;

// ---------------------------------------------------------------------------
// Proptest: random operation sequences with oracle model
//
// Generates random sequences of operations (activate, write pending, start/
// kill receiver, fire hooks) over a pool of 3 sessions. An oracle model
// tracks expected state and predicts the exact outcome of every FireHook
// operation. Any divergence between the real system and the model is a bug.
//
// This catches bugs that only surface in specific multi-session interleavings
// — the kind of concurrent scenarios that are hard to enumerate by hand.
// ---------------------------------------------------------------------------

const ALL_HOOK_TYPES: [HookType; 3] = [HookType::Stop, HookType::PreToolUse, HookType::PostToolUse];
const NUM_SESSIONS: usize = 3;
const SESSION_NAMES: [&str; NUM_SESSIONS] = ["s0", "s1", "s2"];

/// What check_narration communicated back to the agent.
///
/// Re-imported from harness for pattern matching in the oracle.
use super::harness::Outcome;

/// A random operation on the hook state machine.
#[derive(Debug, Clone)]
enum Op {
    /// Simulate `/attend` activation for session `n`.
    Activate(usize),
    /// Write a pending narration file for session `n`.
    WritePending(usize),
    /// Start a fake background receiver (write lock with live PID).
    StartReceiver,
    /// Kill the fake receiver (remove lock file).
    KillReceiver,
    /// Fire a narration hook and check the outcome against the oracle.
    FireHook {
        session: usize,
        hook_type: HookType,
        is_listen: bool,
    },
}

/// Strategy that generates a single random operation.
///
/// Session indices are drawn from `0..NUM_SESSIONS`. Hook types are drawn
/// from the three types that reach `check_narration`. `is_listen` is only
/// set for ToolUse hooks (Stop hooks don't have a tool name).
fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..NUM_SESSIONS).prop_map(Op::Activate),
        2 => (0..NUM_SESSIONS).prop_map(Op::WritePending),
        1 => Just(Op::StartReceiver),
        1 => Just(Op::KillReceiver),
        8 => (0..NUM_SESSIONS, 0..3usize, any::<bool>()).prop_map(|(s, ht_idx, listen)| {
            let hook_type = ALL_HOOK_TYPES[ht_idx];
            // is_listen only meaningful for ToolUse hooks, not Stop.
            let is_listen = listen && hook_type != HookType::Stop;
            Op::FireHook {
                session: s,
                hook_type,
                is_listen,
            }
        }),
    ]
}

/// Oracle model: mirrors the filesystem state that `check_narration` reads
/// and predicts the exact outcome of every hook invocation.
///
/// After each `FireHook`, the model both asserts the prediction and updates
/// itself for side effects (narration archival, SessionMoved ratchet).
struct OracleModel {
    /// Which session index owns narration, if any.
    listening: Option<usize>,
    /// Which sessions have ever run `/attend`.
    activated: [bool; NUM_SESSIONS],
    /// Which sessions have been told about a session steal (ratchet).
    moved_notified: [bool; NUM_SESSIONS],
    /// Whether the receiver process is alive.
    receiver_alive: bool,
    /// Whether each session has undelivered pending narration.
    has_pending: [bool; NUM_SESSIONS],
}

impl OracleModel {
    fn new() -> Self {
        Self {
            listening: None,
            activated: [false; NUM_SESSIONS],
            moved_notified: [false; NUM_SESSIONS],
            receiver_alive: false,
            has_pending: [false; NUM_SESSIONS],
        }
    }

    fn activate(&mut self, session: usize) {
        self.listening = Some(session);
        self.activated[session] = true;
        self.moved_notified[session] = false; // ratchet reset on re-activation
    }

    fn write_pending(&mut self, session: usize) {
        self.has_pending[session] = true;
    }

    fn start_receiver(&mut self) {
        self.receiver_alive = true;
    }

    fn kill_receiver(&mut self) {
        self.receiver_alive = false;
    }

    fn relation(&self, session: usize) -> SessionRelation {
        match self.listening {
            Some(l) if l == session => SessionRelation::Active,
            Some(_) => SessionRelation::Stolen,
            None => SessionRelation::Inactive,
        }
    }

    /// Assert the outcome matches the oracle prediction, then update
    /// model state for any side effects.
    ///
    /// This is a complete oracle: it encodes every reachable branch of
    /// `check_narration` -> `handle_listen_hook` / `handle_general_hook`
    /// -> `general_decision`, including the activation gate and the
    /// SessionMoved ratchet. A mismatch means the code diverges from
    /// the spec.
    fn check_and_update(
        &mut self,
        session: usize,
        hook_type: HookType,
        is_listen: bool,
        outcome: &Outcome,
    ) {
        let relation = self.relation(session);

        // -- Activation gate --
        // Sessions that never ran `/attend` and aren't the active listener
        // are invisible to the hook system. This is the first check in
        // check_narration: if it fires, nothing else runs.
        if relation != SessionRelation::Active && !self.activated[session] {
            self.assert_decision(outcome, &HookDecision::Silent, "activation gate");
            return;
        }

        // -- attend listen PostToolUse --
        // The command already ran. This fires before the relation check
        // in handle_listen_hook, so it applies to Active, Stolen, and
        // Inactive alike (as long as the session was activated).
        if is_listen && hook_type == HookType::PostToolUse {
            self.assert_decision(
                outcome,
                &HookDecision::approve(GuidanceReason::ListenerStarted),
                "listen PostToolUse",
            );
            return;
        }

        // -- Inactive (no listener exists) --
        // Both handle_listen_hook and handle_general_hook/general_decision
        // return Silent for Inactive.
        if relation == SessionRelation::Inactive {
            self.assert_decision(outcome, &HookDecision::Silent, "inactive");
            return;
        }

        // -- Stolen session --
        if relation == SessionRelation::Stolen {
            if is_listen {
                // Anti-livelock: attend listen is blocked for stolen sessions
                // to prevent two sessions bouncing the listener back and forth.
                self.assert_decision(
                    outcome,
                    &HookDecision::block(GuidanceReason::SessionMoved),
                    "stolen attend listen (anti-livelock)",
                );
            } else {
                // General hooks: advisory SessionMoved once, then silent
                // (ratchet). The ratchet prevents the agent from seeing the
                // same "session moved" message on every single tool call.
                if self.moved_notified[session] {
                    self.assert_decision(
                        outcome,
                        &HookDecision::Silent,
                        "stolen general (ratchet suppressed)",
                    );
                } else {
                    self.assert_decision(
                        outcome,
                        &HookDecision::approve(GuidanceReason::SessionMoved),
                        "stolen general (first notification)",
                    );
                    self.moved_notified[session] = true;
                }
            }
            return;
        }

        // -- Active session --
        assert_eq!(relation, SessionRelation::Active);

        if is_listen {
            // attend listen PreToolUse on active session.
            if self.has_pending[session] {
                // Delivery path: pending narration is read, rendered, and
                // delivered in one round trip. Pending files are archived.
                self.assert_narration(outcome, "active listen delivery");
                self.has_pending[session] = false;
            } else if self.receiver_alive {
                // Duplicate listener prevention.
                self.assert_decision(
                    outcome,
                    &HookDecision::block(GuidanceReason::ListenerAlreadyActive),
                    "active listen (receiver alive)",
                );
            } else {
                // No pending, no receiver: let the listener start silently.
                self.assert_decision(
                    outcome,
                    &HookDecision::Silent,
                    "active listen (clean startup)",
                );
            }
        } else {
            // General (non-listen) hook on active session.
            if self.has_pending[session] {
                // Pending narration blocks all hooks to force the agent
                // to run `attend listen` before continuing.
                self.assert_decision(
                    outcome,
                    &HookDecision::block(GuidanceReason::NarrationReady),
                    "active general (pending)",
                );
            } else if self.receiver_alive {
                // Receiver is handling delivery. Nothing to do.
                self.assert_decision(
                    outcome,
                    &HookDecision::Silent,
                    "active general (receiver alive)",
                );
            } else {
                // No receiver, no pending: nudge to start one.
                // Block on Stop (don't exit without a receiver),
                // advisory on ToolUse (let the tool through).
                let expected = match hook_type {
                    HookType::Stop => HookDecision::block(GuidanceReason::StartReceiver),
                    _ => HookDecision::approve(GuidanceReason::StartReceiver),
                };
                self.assert_decision(outcome, &expected, "active general (no receiver)");
            }
        }
    }

    /// Assert the outcome is a specific decision, with model context in
    /// the failure message.
    fn assert_decision(&self, outcome: &Outcome, expected: &HookDecision, label: &str) {
        match outcome {
            Outcome::Decision(d) => assert_eq!(d, expected, "[{label}] model={self:?}"),
            Outcome::Narration(c) => {
                panic!("[{label}] expected {expected:?}, got narration: {c}\nmodel={self:?}")
            }
        }
    }

    /// Assert the outcome is narration delivery (any content).
    fn assert_narration(&self, outcome: &Outcome, label: &str) {
        match outcome {
            Outcome::Narration(_) => {}
            Outcome::Decision(d) => {
                panic!("[{label}] expected narration delivery, got {d:?}\nmodel={self:?}")
            }
        }
    }
}

impl std::fmt::Debug for OracleModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("listening", &self.listening.map(|i| SESSION_NAMES[i]))
            .field("activated", &self.activated)
            .field("moved_notified", &self.moved_notified)
            .field("receiver_alive", &self.receiver_alive)
            .field("has_pending", &self.has_pending)
            .finish()
    }
}

proptest! {
    /// Generate random sequences of 1-40 operations over 3 sessions and
    /// verify every hook outcome matches the oracle model.
    ///
    /// The model is a complete oracle: it predicts the exact decision for
    /// every combination of (relation x activated x pending x receiver x
    /// hook_type x is_listen x moved_notified). Any divergence is a bug,
    /// and proptest shrinking will find the minimal failing sequence.
    #[test]
    fn random_sequences_match_oracle(ops in prop::collection::vec(op_strategy(), 1..40)) {
        let h = TestHarness::new();
        let sessions: Vec<SessionId> = SESSION_NAMES.iter().map(|&s| s.into()).collect();
        let mut model = OracleModel::new();
        let mut receiver_guard: Option<ReceiverGuard> = None;

        for op in &ops {
            match op {
                Op::Activate(s) => {
                    h.activate(&sessions[*s]);
                    model.activate(*s);
                }
                Op::WritePending(s) => {
                    h.write_pending(&sessions[*s], "test narration");
                    model.write_pending(*s);
                }
                Op::StartReceiver => {
                    if receiver_guard.is_none() {
                        receiver_guard = Some(h.fake_receiver());
                        model.start_receiver();
                    }
                }
                Op::KillReceiver => {
                    if receiver_guard.is_some() {
                        receiver_guard = None;
                        model.kill_receiver();
                    }
                }
                Op::FireHook {
                    session,
                    hook_type,
                    is_listen,
                } => {
                    let outcome = h.fire_hook(
                        &sessions[*session],
                        *hook_type,
                        *is_listen,
                        false, // stop_hook_active always false (safety valve
                               // tested separately by exhaustive + scenario tests)
                    );
                    model.check_and_update(*session, *hook_type, *is_listen, &outcome);
                }
            }
        }
    }
}
