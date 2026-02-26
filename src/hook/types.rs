use camino::Utf8PathBuf;

use crate::state::SessionId;

/// Discriminant for which hook event is being processed.
///
/// Passed to `Agent::parse_hook_input` so it can construct the right
/// `HookKind` from the agent's raw input format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookType {
    SessionStart,
    UserPrompt,
    Stop,
    PreToolUse,
    PostToolUse,
    SessionEnd,
}

/// Parsed input from an agent hook invocation.
///
/// Each agent fills this from its own input source (e.g., Claude reads
/// stdin JSON). The `kind` field carries hook-type-specific data.
#[derive(Debug, Default)]
pub struct HookInput {
    pub session_id: Option<SessionId>,
    pub cwd: Option<Utf8PathBuf>,
    pub kind: HookKind,
}

/// Hook-type-specific payload.
#[derive(Debug, Default)]
pub enum HookKind {
    #[default]
    SessionStart,
    UserPrompt {
        prompt: Option<String>,
    },
    Stop {
        stop_hook_active: bool,
    },
    ToolUse {
        /// Bash command string, if the tool is a Bash invocation.
        /// `None` for non-Bash tools or when the agent doesn't provide it.
        bash_command: Option<String>,
    },
}

/// Structured hook decision with semantic variants.
///
/// Produced by `check_narration` / `hook_decision`, consumed by each agent's
/// `attend_result` method to render agent-specific output.
///
/// Narration content is delivered separately via `Agent::deliver_narration`,
/// not through this enum. This enum only carries guidance decisions.
#[derive(Debug, Clone, PartialEq)]
pub enum HookDecision {
    /// No output needed.
    Silent,
    /// Guidance for the agent. The `effect` field (block/approve) is decided
    /// by business logic upstream — the agent renderer should not transform
    /// one into the other.
    Guidance {
        reason: GuidanceReason,
        effect: GuidanceEffect,
    },
}

/// Why the guidance is being issued.
///
/// Each variant carries enough semantic information for agents to render
/// appropriate output — agent-specific strings live in the agent impl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuidanceReason {
    /// Narration is active in a different session.
    SessionMoved,
    /// No background receiver is running: agent should start one.
    StartReceiver,
    /// Narration is pending. The agent should run `attend listen` so its
    /// PreToolUse hook can deliver the content and start a new receiver
    /// in one round trip.
    NarrationReady,
    /// A listener is already active for this session.
    ListenerAlreadyActive,
    /// A listener was just started in the background. Primes the agent to
    /// restart (not read the output) when the task notification arrives.
    ListenerStarted,
    /// Narration was deactivated via `attend listen --stop`.
    Deactivated,
}

/// Whether guidance blocks or approves the current action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidanceEffect {
    /// Block the current action.
    Block,
    /// Approve the current action (guidance is advisory).
    Approve,
}

impl HookDecision {
    pub(super) fn block(reason: GuidanceReason) -> Self {
        Self::Guidance {
            reason,
            effect: GuidanceEffect::Block,
        }
    }

    pub(super) fn approve(reason: GuidanceReason) -> Self {
        Self::Guidance {
            reason,
            effect: GuidanceEffect::Approve,
        }
    }
}
