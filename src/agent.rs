mod claude;
// <-- Add new agent modules here

use anyhow::Context;
use camino::Utf8PathBuf;

use crate::hook::HookDecision;
pub use crate::hook::{HookInput, HookType};
use crate::state::{EditorState, SessionId};

/// A backend that can parse input, render output, and install/uninstall hooks.
pub trait Agent: Sync {
    /// CLI name (e.g., "claude").
    fn name(&self) -> &'static str;

    // --- Input ---

    /// Parse hook input from agent-specific source.
    fn parse_hook_input(&self, hook_type: HookType) -> HookInput;

    // --- Output (one per hook) ---

    /// Emit session-start output. `is_listening` = narration active for this session.
    fn session_start(&self, input: &HookInput, is_listening: bool) -> anyhow::Result<()>;
    /// Emit editor context when state has changed.
    fn editor_context(&self, state: &EditorState) -> anyhow::Result<()>;
    /// Emit /attend activation response.
    fn attend_activate(&self, session_id: &SessionId) -> anyhow::Result<()>;
    /// Emit /unattend deactivation response.
    fn attend_deactivate(&self, session_id: &SessionId) -> anyhow::Result<()>;
    /// Deliver narration content and approve the `attend listen` tool call.
    ///
    /// Called from the `attend listen` PreToolUse path — the sole content
    /// delivery mechanism. The agent formats the narration for its output
    /// protocol and emits an "approve" so the listener starts in the same
    /// round trip.
    fn deliver_narration(&self, content: &str) -> anyhow::Result<()>;
    /// Emit hook decision. `hook_type` controls output format (e.g.,
    /// PreToolUse approves `StartReceiver` rather than blocking).
    fn attend_result(&self, decision: &HookDecision, hook_type: HookType) -> anyhow::Result<()>;

    // --- Install ---

    /// Install hooks into agent settings.
    fn install(&self, bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()>;
    /// Remove hooks from agent settings.
    fn uninstall(&self, project: Option<Utf8PathBuf>) -> anyhow::Result<()>;
}

/// All registered agent backends.
pub const AGENTS: &[&'static dyn Agent] = &[
    &claude::Claude,
    // <-- Add new agents here
];

/// Look up an agent by CLI name.
pub fn backend_by_name(name: &str) -> Option<&'static dyn Agent> {
    AGENTS.iter().find(|a| a.name() == name).copied()
}

/// Determine the binary command string for hook installation.
pub(crate) fn resolve_bin_cmd(dev: bool) -> anyhow::Result<String> {
    let bin_name = std::env::args()
        .next()
        .and_then(|a| Utf8PathBuf::from(a).file_name().map(|f| f.to_string()))
        .unwrap_or_else(|| "attend".to_string());

    if dev {
        let exe = std::env::current_exe().context("cannot determine current exe path")?;
        let exe = Utf8PathBuf::try_from(exe)
            .map_err(|e| anyhow::anyhow!("non-UTF-8 exe path: {}", e.into_path_buf().display()))?;
        Ok(exe.into_string())
    } else {
        which::which(&bin_name)
            .map(|_| bin_name)
            .map_err(|e| anyhow::anyhow!("cannot find binary on PATH: {e}"))
    }
}

/// Install hooks into the agent's settings file.
pub fn install(agent_name: &str, project: Option<Utf8PathBuf>, dev: bool) -> anyhow::Result<()> {
    let agent = backend_by_name(agent_name)
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {agent_name}"))?;
    let bin_cmd = resolve_bin_cmd(dev)?;
    agent.install(&bin_cmd, project)
}

/// Remove hooks from the agent's settings file.
pub fn uninstall(agent_name: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
    let agent = backend_by_name(agent_name)
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {agent_name}"))?;
    agent.uninstall(project)
}
