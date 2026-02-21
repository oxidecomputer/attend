mod input;
mod output;
mod settings;

use camino::Utf8PathBuf;

use super::Agent;
use crate::hook::{HookDecision, HookInput, HookType};
use crate::state::{EditorState, SessionId};

/// Claude Code agent backend.
pub struct Claude;

impl Agent for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn parse_hook_input(&self, hook_type: HookType) -> HookInput {
        input::parse(hook_type)
    }

    fn session_start(&self, input: &HookInput, is_listening: bool) -> anyhow::Result<()> {
        output::session_start(input, is_listening)
    }

    fn editor_context(&self, state: &EditorState) -> anyhow::Result<()> {
        output::editor_context(state)
    }

    fn attend_activate(&self, session_id: &SessionId) -> anyhow::Result<()> {
        output::attend_activate(session_id)
    }

    fn deliver_narration(&self, content: &str) -> anyhow::Result<()> {
        output::deliver_narration(content)
    }

    fn attend_result(&self, decision: &HookDecision, hook_type: HookType) -> anyhow::Result<()> {
        output::attend_result(decision, hook_type)
    }

    fn install(&self, bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        settings::install(bin_cmd, project)
    }

    fn uninstall(&self, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        settings::uninstall(project)
    }
}
