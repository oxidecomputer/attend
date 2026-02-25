use super::super::*;
use super::harness::TestHarness;
use crate::state::SessionId;

const ALL_HOOK_TYPES: [HookType; 3] = [HookType::Stop, HookType::PreToolUse, HookType::PostToolUse];

// ---------------------------------------------------------------------------
// Scenario tests — each exercises check_narration against real filesystem
// state in a temp directory, simulating multi-session scenarios.
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
/// **Flow**: Session activates -> first hook nudges to start receiver ->
/// receiver starts -> hooks go silent -> narration arrives -> hook blocks
/// with NarrationReady -> `attend listen` delivers content -> hooks go
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
/// **Flow**: A activates -> B activates (steals) -> A's hooks get advisory
/// SessionMoved (once) then go silent -> B's hooks behave as the active
/// session -> A's `attend listen` is blocked (anti-livelock).
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
/// **Flow**: A activates -> B steals -> A gets SessionMoved (once) -> A
/// re-activates with /attend -> B steals again -> A gets SessionMoved
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
/// **Flow**: Active session, no receiver, Stop fires -> Block(StartReceiver).
/// Claude Code re-invokes with stop_hook_active=true -> Silent (safety valve).
/// This prevents: block -> re-invoke -> block -> re-invoke -> ...
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
    h.write_pending(&s, "second narration");

    // attend listen delivers both.
    let out = h.fire_hook(&s, HookType::PreToolUse, true, false);
    TestHarness::assert_narration(&out, "first narration");
    TestHarness::assert_narration(&out, "second narration");

    // After delivery, no more pending.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::StartReceiver));
}

/// **Scenario**: Livelock variant A (regression): undeliverable pending
/// files + receiver alive.
///
/// **Before fix**: Pending files existed but their content was filtered
/// out by cwd. General hooks blocked with NarrationReady. `attend listen`
/// couldn't deliver anything but didn't archive the files. With a receiver
/// alive, it blocked with ListenerAlreadyActive. The pending files
/// persisted, so the cycle repeated forever (livelock).
///
/// **After fix**: The general hook detects that pending files are
/// undeliverable, archives them, and reports no pending. The agent is
/// never blocked for content that can't be delivered.
#[test]
fn livelock_variant_a_undeliverable_pending_plus_receiver() {
    let h = TestHarness::new();
    let s: SessionId = "session-1".into();
    h.activate(&s);
    let _receiver = h.fake_receiver();

    // Write pending content that will be filtered out during delivery.
    h.write_undeliverable_pending(&s);

    // General hook: detects undeliverable files, archives them, reports
    // no pending. Receiver is alive, so result is Silent.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);

    // Subsequent hooks remain clean: no pending, receiver alive.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::Silent);
}

/// **Scenario**: Livelock variant B (regression): undeliverable pending
/// files, no receiver.
///
/// **Before fix**: Pending files existed but their content was filtered
/// out. No receiver was alive. General hook blocked with NarrationReady.
/// `attend listen` approved silently but didn't archive files. The
/// receiver saw files and exited with REDISPATCH_MSG. Agent restarted
/// listener, general hook blocked again. Infinite cycle.
///
/// **After fix**: The general hook detects that pending files are
/// undeliverable, archives them, and reports no pending. The normal
/// "start a receiver" nudge is emitted instead.
#[test]
fn livelock_variant_b_undeliverable_pending_no_receiver() {
    let h = TestHarness::new();
    let s: SessionId = "session-1".into();
    h.activate(&s);

    // Write pending content that will be filtered out during delivery.
    h.write_undeliverable_pending(&s);

    // General hook: detects undeliverable files, archives them, reports
    // no pending. No receiver, so nudges to start one.
    let out = h.fire_hook(&s, HookType::PreToolUse, false, false);
    TestHarness::assert_decision(&out, &HookDecision::approve(GuidanceReason::StartReceiver));

    // Subsequent hooks remain clean: no pending, no receiver.
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
