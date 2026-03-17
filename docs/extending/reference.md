# Extending reference

Technical reference for the traits, types, and templates used by `attend`'s
integration backends. See the individual how-to guides for step-by-step
instructions on adding a new
[agent](agents.md), [editor](editors.md),
[shell](shells.md), or [browser](browsers.md).

## Agent trait

Defined in `src/agent/mod.rs`.

### Input

| Method | Purpose |
|--------|---------|
| `name()` | CLI name, e.g. `"claude"` |
| `parse_hook_input(hook_type)` | Read from whatever source the agent provides (stdin, env, etc.) and return a `HookInput` |

### Output

| Method | Purpose |
|--------|---------|
| `session_start(input, is_listening)` | Emit instructions for the agent session. If `is_listening`, also emit narration instructions. |
| `editor_context(state)` | Render editor state (open files, cursors, selections) to stdout |
| `attend_activate(session_id)` | Acknowledge narration activation |
| `attend_deactivate(session_id)` | Acknowledge narration deactivation |
| `deliver_narration(content)` | Deliver narration content. Called during `attend listen` PreToolUse. Should also emit "approve" so the listener starts in the same round trip. |
| `attend_result(decision, hook_type)` | Render a hook decision to stdout. `hook_type` controls whether guidance blocks or approves. |

### Install/uninstall

| Method | Purpose |
|--------|---------|
| `install(bin_cmd, project)` | Write hook commands and a skill into the agent's settings |
| `uninstall(project)` | Remove hook entries and skill from the agent's settings |

### `HookInput`

```rust
pub struct HookInput {
    pub session_id: Option<SessionId>,
    pub cwd: Option<Utf8PathBuf>,
    pub kind: HookKind,
}

pub enum HookKind {
    SessionStart,
    UserPrompt { prompt: Option<String> },
    Stop { stop_hook_active: bool },
    ToolUse { bash_command: Option<String> },
}
```

Claude reads this from JSON on stdin. Other agents may obtain it differently.

### `HookDecision`

The hook orchestrator computes a semantic decision that each agent renders
in its own format:

| Variant | Meaning |
|---------|---------|
| `Silent` | No output needed |
| `Guidance(reason, effect)` | Operational guidance with block or approve effect |

Guidance reasons:

| Reason | Meaning |
|--------|---------|
| `SessionMoved` | Narration is active in a different session |
| `StartReceiver` | No receiver running: agent should start one |
| `NarrationReady` | Pending narration: agent should run `attend listen` |
| `ListenerAlreadyActive` | A listener is already running for this session |
| `ListenerStarted` | A listener was just started in the background |
| `Deactivated` | Narration was deactivated via `attend listen --stop` |

### Hook events

Five hook events drive the agent integration:

| Hook | When it fires | What the orchestrator does |
|------|---------------|---------------------------|
| `SessionStart` | Session start, clear, compact | Clear per-session cache, auto-upgrade hooks, emit instructions |
| `UserPromptSubmit` | Before each user prompt | Detect `/attend`, or query + deduplicate + emit editor context |
| `Stop` | Session stops | Require the agent to call `attend listen` if narration pending |
| `PreToolUse` | Before each tool call | Require the agent to call `attend listen` if narration pending |
| `PostToolUse` | After each tool call | Require the agent to call `attend listen` if narration pending |

The `PreToolUse` hook on `attend listen` is the **sole place** where narration
is actually delivered to the agent. This forces a linear sequence: the agent
keeps exactly one background listener running and receives narration before
doing anything else.

## Editor trait

Defined in `src/editor/mod.rs`.

| Method | Required | Purpose |
|--------|----------|---------|
| `name()` | yes | CLI name, e.g. `"zed"` |
| `query()` | yes | Return open tabs with byte-offset selections |
| `install_narration(bin_cmd)` | no | Install narration hotkey/task integration |
| `uninstall_narration()` | no | Remove narration integration |
| `check_narration()` | no | Return diagnostic warnings (empty = healthy) |

The narration methods have default implementations that return an error or
empty result. Implement them if the editor supports voice narration hotkeys.

### `RawEditor`

One open tab/pane with a single cursor or selection:

| Field | Type | Meaning |
|-------|------|---------|
| `path` | `PathBuf` | Absolute file path |
| `sel_start` | `Option<i64>` | Byte offset of selection/cursor start |
| `sel_end` | `Option<i64>` | Byte offset of selection/cursor end |

A cursor is `sel_start == sel_end`. Return `None` for both when the editor
doesn't expose selection data.

## Shell trait

Defined in `src/shell.rs`.

| Method | Required | Purpose |
|--------|----------|---------|
| `name()` | yes | CLI name, e.g. `"fish"` |
| `install_hooks(bin_cmd)` | yes | Write hook script that calls `attend shell-hook` |
| `uninstall_hooks()` | yes | Remove the hook script |
| `install_completions(bin_cmd)` | yes | Generate and write tab completions |
| `uninstall_completions()` | yes | Remove the completions file |
| `check()` | no | Return diagnostic warnings (empty = healthy) |

## Browser trait

Defined in `src/browser.rs`.

| Method | Required | Purpose |
|--------|----------|---------|
| `name()` | yes | CLI name, e.g. `"firefox"` |
| `install(bin_cmd)` | yes | Install native messaging manifest + extension |
| `uninstall()` | yes | Remove manifest and extension files |

## Shared message templates

Templates in `src/agent/messages/` are shared across all agents:

| Template | Purpose |
|----------|---------|
| `editor_context_instructions.txt` | How to interpret `<editor-context>` tags and use `attend look`. Placeholder: `{bin_cmd}`. |
| `narration_protocol.md` | Full narration protocol: silence requirement, delivery paths, receiver restart behavior, `<narration>` tag format, cursor-only handling, `include_dirs`. Placeholder: `{start_skill}`. |
| `narration_pause.txt` | "Pause and consider narration before using tools" |
| `activate_response.txt` | Confirmation when narration is activated |
| `deactivate_response.txt` | Confirmation when narration is deactivated |
| `guidance_session_moved.txt` | "Narration moved to another session" |
| `guidance_start_receiver.txt` | "Start the receiver" nudge |
| `guidance_listener_active.txt` | "Listener already running" |
| `guidance_deactivated.txt` | "Narration deactivated" |

Agent-specific templates go in `src/agent/<name>/messages/`.

## Supporting infrastructure

### Auto-upgrade

On each `SessionStart` hook, `attend` checks whether the running binary version
matches the version that installed the hooks (`~/.cache/attend/version.json`).
On mismatch, it automatically reinstalls all previously registered agents and
editors.

### Project path tracking

`attend install --project /path/to/project` records the path in
`InstallMeta.project_paths`. `attend uninstall` (without `--project`) cleans up
all tracked project paths, preventing stale project-local config from
accumulating.

### Narration delivery paths

Narration reaches the agent through two paths:

1. **Hook delivery** (non-blocking): The Stop, PreToolUse, and PostToolUse
   hooks interrupt the agent whenever pending narration is available, preventing
   it from ending its turn or invoking further tools until it receives the
   narration by re-invoking `attend listen`.

2. **Background receiver** (blocking): The agent starts `attend listen` in the
   background. The receiver polls for pending files and exits when they arrive
   so the agent can restart it for the next narration. Narration is delivered
   exclusively by the PreToolUse hook on `Bash(attend listen)`.

### Receiver output protocol

The `attend listen` receiver is agent-agnostic. It uses a standard output
protocol based on XML tags:

- Narration content is wrapped in `<narration>` tags
- Operational instructions (restart, conflict) are wrapped in
  `<system-instruction>` tags

Each agent's instructions teach its LLM to expect this format. If an agent's
LLM requires fundamentally different framing, it can implement a custom
listener.
