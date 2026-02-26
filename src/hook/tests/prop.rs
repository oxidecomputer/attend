use proptest::prelude::*;

use super::super::*;
use super::harness::{ListenVariant, ReceiverGuard, TestHarness};
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

/// Which listen variant to simulate.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ListenKind {
    /// Not an `attend listen` command.
    None,
    /// `attend listen` (start/wait).
    Listen,
    /// `attend listen --stop` (deactivation).
    ListenStop,
}

/// A random operation on the hook state machine.
#[derive(Debug, Clone)]
enum Op {
    /// Simulate `/attend` activation for session `n`.
    Activate(usize),
    /// Write a pending narration file for session `n`.
    WritePending(usize),
    /// Write a pending narration file whose content will be filtered out
    /// during delivery (path outside cwd). Exercises the livelock bug path.
    WriteUndeliverablePending(usize),
    /// Start a fake background receiver (write lock with live PID).
    StartReceiver,
    /// Kill the fake receiver (remove lock file).
    KillReceiver,
    /// Fire a narration hook and check the outcome against the oracle.
    FireHook {
        session: usize,
        hook_type: HookType,
        listen_kind: ListenKind,
    },
}

/// Strategy that generates a single random operation.
///
/// Session indices are drawn from `0..NUM_SESSIONS`. Hook types are drawn
/// from the three types that reach `check_narration`. Listen variants are
/// drawn only for ToolUse hooks (Stop hooks don't have a tool name).
fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..NUM_SESSIONS).prop_map(Op::Activate),
        2 => (0..NUM_SESSIONS).prop_map(Op::WritePending),
        2 => (0..NUM_SESSIONS).prop_map(Op::WriteUndeliverablePending),
        1 => Just(Op::StartReceiver),
        1 => Just(Op::KillReceiver),
        8 => (0..NUM_SESSIONS, 0..3usize, 0..3usize).prop_map(|(s, ht_idx, listen_idx)| {
            let hook_type = ALL_HOOK_TYPES[ht_idx];
            // listen variants only meaningful for ToolUse hooks, not Stop.
            let listen_kind = if hook_type == HookType::Stop {
                ListenKind::None
            } else {
                match listen_idx {
                    0 => ListenKind::None,
                    1 => ListenKind::Listen,
                    _ => ListenKind::ListenStop,
                }
            };
            Op::FireHook {
                session: s,
                hook_type,
                listen_kind,
            }
        }),
    ]
}

/// Whether a session has pending narration, and if so, whether it's
/// deliverable (contains renderable content after cwd filtering).
#[derive(Debug, Clone, Copy, PartialEq)]
enum PendingKind {
    /// No pending files.
    None,
    /// Files exist but all content will be filtered out by cwd.
    Undeliverable,
    /// Files exist with at least some deliverable content.
    Deliverable,
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
    /// What kind of pending narration each session has.
    pending: [PendingKind; NUM_SESSIONS],
}

impl OracleModel {
    fn new() -> Self {
        Self {
            listening: None,
            activated: [false; NUM_SESSIONS],
            moved_notified: [false; NUM_SESSIONS],
            receiver_alive: false,
            pending: [PendingKind::None; NUM_SESSIONS],
        }
    }

    fn activate(&mut self, session: usize) {
        self.listening = Some(session);
        self.activated[session] = true;
        self.moved_notified[session] = false; // ratchet reset on re-activation
    }

    fn write_pending(&mut self, session: usize) {
        // Deliverable content: overrides any prior state.
        self.pending[session] = PendingKind::Deliverable;
    }

    fn write_undeliverable_pending(&mut self, session: usize) {
        // Only set to Undeliverable if nothing is pending yet.
        // If deliverable content already exists, the combined set is
        // still deliverable (read_pending finds the deliverable events).
        if self.pending[session] == PendingKind::None {
            self.pending[session] = PendingKind::Undeliverable;
        }
    }

    fn start_receiver(&mut self) {
        self.receiver_alive = true;
    }

    fn kill_receiver(&mut self) {
        self.receiver_alive = false;
    }

    fn has_pending(&self, session: usize) -> bool {
        self.pending[session] != PendingKind::None
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
    /// `check_narration` -> `handle_listen_hook` / `handle_unlisten_hook`
    /// / `handle_general_hook` -> `general_decision`, including the
    /// activation gate and the SessionMoved ratchet. A mismatch means
    /// the code diverges from the spec.
    fn check_and_update(
        &mut self,
        session: usize,
        hook_type: HookType,
        listen_kind: ListenKind,
        outcome: &Outcome,
    ) {
        let relation = self.relation(session);

        // -- Activation gate --
        // Sessions that never ran `/attend` and aren't the active listener
        // are invisible to the hook system, UNLESS they're running
        // `attend listen` (auto-claim path).
        if relation != SessionRelation::Active
            && !self.activated[session]
            && listen_kind != ListenKind::Listen
        {
            self.assert_decision(outcome, &HookDecision::Silent, "activation gate");
            return;
        }

        // -- attend listen --stop --
        if listen_kind == ListenKind::ListenStop {
            if hook_type == HookType::PostToolUse {
                // Command already ran, approve silently.
                self.assert_decision(outcome, &HookDecision::Silent, "listen --stop PostToolUse");
                return;
            }
            // PreToolUse
            match relation {
                SessionRelation::Active => {
                    // Deactivate: remove listening file.
                    self.listening = None;
                    self.assert_decision(
                        outcome,
                        &HookDecision::approve(GuidanceReason::Deactivated),
                        "listen --stop active",
                    );
                }
                SessionRelation::Stolen | SessionRelation::Inactive => {
                    self.assert_decision(
                        outcome,
                        &HookDecision::block(GuidanceReason::SessionMoved),
                        "listen --stop non-owner",
                    );
                }
            }
            return;
        }

        // -- attend listen PostToolUse --
        // The command already ran. This fires before the relation check
        // in handle_listen_hook, so it applies to Active, Stolen, and
        // Inactive alike (as long as the session was activated).
        if listen_kind == ListenKind::Listen && hook_type == HookType::PostToolUse {
            self.assert_decision(
                outcome,
                &HookDecision::approve(GuidanceReason::ListenerStarted),
                "listen PostToolUse",
            );
            return;
        }

        // -- Inactive (no listener exists) --
        // Both handle_listen_hook and handle_general_hook/general_decision
        // return Silent for Inactive, except attend listen which auto-claims.
        if relation == SessionRelation::Inactive {
            if listen_kind == ListenKind::Listen && hook_type == HookType::PreToolUse {
                // Auto-claim: activate the session.
                self.activate(session);
                self.assert_activation(outcome, "inactive listen auto-claim");
            } else {
                self.assert_decision(outcome, &HookDecision::Silent, "inactive");
            }
            return;
        }

        // -- Stolen session --
        if relation == SessionRelation::Stolen {
            if listen_kind == ListenKind::Listen {
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

        if listen_kind == ListenKind::Listen {
            // attend listen PreToolUse on active session.
            match self.pending[session] {
                PendingKind::Deliverable => {
                    // Delivery path: pending narration is read, rendered,
                    // and delivered in one round trip. All files archived.
                    self.assert_narration(outcome, "active listen delivery");
                    self.pending[session] = PendingKind::None;
                }
                PendingKind::Undeliverable => {
                    // Files exist but read_pending returns None (content
                    // filtered out). The fix archives them anyway, clearing
                    // the pending state. Then falls through to receiver/
                    // silent check.
                    self.pending[session] = PendingKind::None;
                    if self.receiver_alive {
                        self.assert_decision(
                            outcome,
                            &HookDecision::block(GuidanceReason::ListenerAlreadyActive),
                            "active listen (undeliverable, receiver alive)",
                        );
                    } else {
                        self.assert_decision(
                            outcome,
                            &HookDecision::Silent,
                            "active listen (undeliverable, clean startup)",
                        );
                    }
                }
                PendingKind::None => {
                    if self.receiver_alive {
                        // Duplicate listener prevention.
                        self.assert_decision(
                            outcome,
                            &HookDecision::block(GuidanceReason::ListenerAlreadyActive),
                            "active listen (receiver alive)",
                        );
                    } else {
                        // No pending, no receiver: let the listener start.
                        self.assert_decision(
                            outcome,
                            &HookDecision::Silent,
                            "active listen (clean startup)",
                        );
                    }
                }
            }
        } else {
            // General (non-listen) hook on active session.
            //
            // Undeliverable pending is cleaned up eagerly: the general
            // hook archives filtered-out files and treats them as absent.
            // Only deliverable pending triggers NarrationReady.
            if self.pending[session] == PendingKind::Undeliverable {
                self.pending[session] = PendingKind::None;
            }
            if self.has_pending(session) {
                // Deliverable pending blocks all hooks to force the agent
                // to run `attend listen` first.
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
            Outcome::Activation => {
                panic!("[{label}] expected {expected:?}, got activation\nmodel={self:?}")
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
            Outcome::Activation => {
                panic!("[{label}] expected narration delivery, got activation\nmodel={self:?}")
            }
        }
    }

    /// Assert the outcome is an activation (auto-claim).
    fn assert_activation(&self, outcome: &Outcome, label: &str) {
        match outcome {
            Outcome::Activation => {}
            Outcome::Decision(d) => {
                panic!("[{label}] expected activation, got {d:?}\nmodel={self:?}")
            }
            Outcome::Narration(c) => {
                panic!("[{label}] expected activation, got narration: {c}\nmodel={self:?}")
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
            .field("pending", &self.pending)
            .finish()
    }
}

proptest! {
    /// Generate random sequences of 1-40 operations over 3 sessions and
    /// verify every hook outcome matches the oracle model.
    ///
    /// The model is a complete oracle: it predicts the exact decision for
    /// every combination of (relation x activated x pending x receiver x
    /// hook_type x listen_kind x moved_notified). Any divergence is a bug,
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
                Op::WriteUndeliverablePending(s) => {
                    h.write_undeliverable_pending(&sessions[*s]);
                    model.write_undeliverable_pending(*s);
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
                    listen_kind,
                } => {
                    let variant = match listen_kind {
                        ListenKind::None => ListenVariant::None,
                        ListenKind::Listen => ListenVariant::Listen,
                        ListenKind::ListenStop => ListenVariant::ListenStop,
                    };
                    let outcome = h.fire_hook_ext(
                        &sessions[*session],
                        *hook_type,
                        variant,
                        false, // stop_hook_active always false (safety valve
                               // tested separately by exhaustive + scenario tests)
                    );
                    model.check_and_update(*session, *hook_type, *listen_kind, &outcome);
                }
            }
        }
    }

    /// **Progress property**: no sequence of general hook -> attend listen ->
    /// general hook can produce NarrationReady twice without new content
    /// being written between the two general hooks.
    ///
    /// This is the livelock invariant. If NarrationReady fires, the agent
    /// runs `attend listen`. After that, pending must be cleared (either
    /// delivered or archived as undeliverable). A subsequent general hook
    /// that returns NarrationReady again means the agent is stuck.
    ///
    /// The property also checks that undeliverable pending never triggers
    /// NarrationReady in the first place: the general hook should detect
    /// undeliverable files, archive them, and report no pending.
    #[test]
    fn no_livelock_possible(
        has_deliverable in any::<bool>(),
        has_undeliverable in any::<bool>(),
        receiver_alive in any::<bool>(),
    ) {
        // Only interesting when there's some pending content.
        prop_assume!(has_deliverable || has_undeliverable);

        let h = TestHarness::new();
        let s: SessionId = "progress-test".into();
        h.activate(&s);

        // Set up receiver state.
        let _receiver = if receiver_alive {
            Some(h.fake_receiver())
        } else {
            None
        };

        // Write pending content based on the flags.
        if has_deliverable {
            h.write_pending(&s, "deliverable content");
        }
        if has_undeliverable {
            h.write_undeliverable_pending(&s);
        }

        // Step 1: General hook. If only undeliverable content exists,
        // it should be archived and NOT trigger NarrationReady.
        let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
        let first_was_narration_ready = matches!(
            &out,
            Outcome::Decision(HookDecision::Guidance {
                reason: GuidanceReason::NarrationReady,
                ..
            })
        );

        if !has_deliverable {
            // Undeliverable-only pending must not trigger NarrationReady.
            assert!(
                !first_was_narration_ready,
                "undeliverable pending should not trigger NarrationReady. \
                 receiver_alive={receiver_alive}"
            );
            return Ok(()); // Nothing more to check.
        }

        // Deliverable pending: NarrationReady should fire.
        assert!(
            first_was_narration_ready,
            "deliverable pending should trigger NarrationReady"
        );

        // Step 2: Agent follows guidance, runs attend listen.
        let _out = h.fire_hook(&s, HookType::PreToolUse, true, false);

        // Step 3: The critical progress check. No NarrationReady after
        // attend listen consumed the pending content.
        let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
        match &out {
            Outcome::Decision(d) => {
                assert_ne!(
                    d,
                    &HookDecision::block(GuidanceReason::NarrationReady),
                    "LIVELOCK: attend listen did not clear pending narration. \
                     has_deliverable={has_deliverable}, has_undeliverable={has_undeliverable}, \
                     receiver_alive={receiver_alive}"
                );
            }
            Outcome::Narration(_) => {
                panic!("general hook should not deliver narration");
            }
            Outcome::Activation => {
                panic!("general hook should not activate");
            }
        }
    }
}
