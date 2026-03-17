# How to add a new agent

An agent integration has two sides: **hook orchestration** (shared logic in
`hook/` that handles session lifecycle, caching, and narration delivery) and
**agent-specific rendering** (how each agent parses input and formats output).

The `Agent` trait in `src/agent/mod.rs` covers the agent-specific side. The
shared orchestration lives in `hook/` and calls into the trait. See [extending
reference](reference.md#agent-trait) for the full trait API and
[extending reference](reference.md#hook-events) for the hook event
table.

## 1. Create the agent module

Create `src/agent/<name>/mod.rs` implementing the `Agent` trait:

```rust
use camino::Utf8PathBuf;

use super::Agent;
use crate::hook::{HookDecision, HookInput, HookType};
use crate::state::{EditorState, SessionId};

pub struct Name;

impl Agent for Name {
    fn name(&self) -> &'static str { "<name>" }

    fn parse_hook_input(&self, hook_type: HookType) -> HookInput {
        // Read from whatever source this agent provides (stdin, env, etc.)
        HookInput::default()
    }

    fn session_start(&self, input: &HookInput, is_listening: bool) -> anyhow::Result<()> {
        // Emit instructions for the agent session.
        Ok(())
    }

    fn editor_context(&self, state: &EditorState) -> anyhow::Result<()> {
        // Render editor state to stdout.
        Ok(())
    }

    fn attend_activate(&self, session_id: &SessionId) -> anyhow::Result<()> {
        Ok(())
    }

    fn attend_deactivate(&self, session_id: &SessionId) -> anyhow::Result<()> {
        Ok(())
    }

    fn deliver_narration(&self, content: &str) -> anyhow::Result<()> {
        // Deliver narration content to the agent.
        Ok(())
    }

    fn attend_result(&self, decision: &HookDecision, hook_type: HookType) -> anyhow::Result<()> {
        // Render the hook decision to stdout.
        Ok(())
    }

    fn install(&self, bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        Ok(())
    }

    fn uninstall(&self, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        Ok(())
    }
}
```

See `src/agent/claude/` for a complete implementation. Key points:

- **Input**: Claude reads JSON from stdin. Other agents may obtain `HookInput`
  differently.
- **Output**: Claude emits JSON to stdout. Other agents may use different
  formats.
- **Install**: Claude writes to `~/.claude/settings.json` (global) or
  `.claude/settings.local.json` (project). Other agents will have settings
  elsewhere.
- **Idempotency**: `install()` must be safe to call repeatedly.
- **Non-interference**: Installation must not disturb other settings and tools.

## 2. Register the backend

In `src/agent/mod.rs`, add the module and register it:

```rust
mod claude;
mod <name>;
```

```rust
pub const AGENTS: &[&'static dyn Agent] = &[
    &claude::Claude,
    &<name>::Name,
];
```

The CLI (`hook --agent <name>`, `install --agent <name>`, etc.) is built
automatically from the registered backends.

## 3. Add agent-specific instructions

Use the shared templates in `src/agent/messages/` for protocol content (see
[extending reference](reference.md#shared-message-templates)). Add
agent-specific templates in `src/agent/<name>/messages/` for activation and
execution instructions that explain how to run `attend listen` in your agent's
execution model.

At minimum, your agent should:

1. On session start: emit `editor_context_instructions.txt` so the agent knows
   how to interpret editor context.
2. On narration activation: emit `activate_response.txt` plus your own
   instructions explaining how to run `attend listen` in the background.
3. On narration deactivation: emit `deactivate_response.txt`.
4. On session start with `is_listening = true`: re-emit narration instructions
   so the agent restarts the receiver after context compaction or clear.

## Checklist

- [ ] `src/agent/<name>/mod.rs` — `pub struct Name` + `impl Agent for Name`
- [ ] `src/agent/mod.rs` — `mod <name>;` declaration
- [ ] `src/agent/mod.rs` — add `&<name>::Name` to `AGENTS`
- [ ] Use shared templates from `src/agent/messages/` for protocol content
- [ ] Add agent-specific templates for activation and execution instructions
- [ ] Test hook install/uninstall round-trips cleanly
