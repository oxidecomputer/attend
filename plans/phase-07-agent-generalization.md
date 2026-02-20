# Phase 7: Agent Trait Generalization

**Dependencies**: Phase 3 (modules reorged), Phase 5 (error handling clean).
**Effort**: Medium | **Risk**: Medium

---

## 7.1 Refactor hook logic into generic + agent-specific

- Agent trait provides:
  - `fn parse_hook_context(&self, event: &str, stdin: &str) -> HookContext` (session_id, cwd)
  - `fn format_hook_output(&self, state: &EditorState) -> String`
  - `fn wrap_system_message(&self, content: &str) -> String`
- `hook.rs` owns shared logic: config loading, state resolution, dedup, stop-active detection
- Claude implementation: parse JSON from stdin, emit `<system-reminder>` tags

## 7.2 Split narration instructions

- Shared protocol template: what `<narration>` tags mean, listen/stop lifecycle, code/diff interleaving
- Agent-specific snippets: "Bash with `run_in_background: true`", `description: "..."`, tool invocation patterns
- Agent trait method provides the agent-specific fragments; shared template lives in common location

## 7.3 Track project-specific installations

- When `install(project: Some(path))` is called, record the installation location
- `uninstall` without a path flag should find and clean up project-specific installs
- Prevent forgetting project-local hooks

## 7.4 Research skill format generalization

- Check if Cursor, Windsurf, or other agent harnesses have a skill/command format
- Determine what's shared vs. Claude-specific in the skill body
- Design the templating if cross-agent skills are feasible

---

## Verification

- All existing hook tests pass unchanged (the refactor preserves behavior)
- Manual test: full `/attend` -> narrate -> stop flow works identically to before
- The Claude agent implementation is the only concrete impl; the trait is the new abstraction
- Adding a hypothetical second agent requires implementing only the trait methods, not touching hook.rs core logic (verify by inspection)
