# Phase 8: UX Improvements

**Dependencies**: Phase 4 (deps available), Phase 6 (daemon restructured).
**Effort**: Medium | **Risk**: Low-Medium

---

## 8.1 Model download during `/attend` activation

- When `attend listen` detects model isn't downloaded, download with progress output
- Agent sees and relays: "Downloading Parakeet model (1.2 GB)..."
- Use `indicatif` for progress bar (visible in agent's stdout capture)
- Subsequent runs skip entirely (model already present)

## 8.2 Auto-cleanup with configurable retention

- New config field: `archive_retention = "7d"` (default, e.g., 7 days; `"forever"` to disable)
- After each `archive_pending()`, prune archives older than retention
- `attend narrate clean` still exists for manual use

## 8.3 Cross-platform keybindings and user-selectable keybindings

- `cmd` is macOS-specific; should be `super` on Linux
- Check Zed's documentation for correct modifier on each platform
- Possibly: `attend editor install-keybindings --editor zed` as a separate command axis (keybinding install separate from agent install)
- Allow users to specify which keybindings to install rather than forcing defaults

## 8.4 Elided line ranges in narration output

- Change `// ... (34 lines omitted)` -> `// ... (lines 45-78 omitted)`
- Makes it trivially actionable: agent can `Read` exactly those lines without arithmetic

## 8.5 Context line tuning for highlights

- Currently 5 lines before/after — evaluate whether this is excessive
- Consider making configurable or reducing default
- Measure token overhead

## 8.6 Check parakeet-rs upstream for CTC timestamp fix

- See if there's a newer release of `parakeet-rs` beyond 0.3
- Not blocking (we use TDT mode which works), but good hygiene

## 8.7 Narration quality: reduce cursor-only noise

- Add a dwell threshold: only emit cursor-only snapshots when the cursor rests at a position for >500ms (or similar). Rapid scanning generates many low-value cursor positions.
- For cursor-only events that do get emitted, include 1-2 surrounding lines of code instead of bare `// src/foo.rs:42\n|` position. A bare position requires `attend look` to interpret, which can't be done mid-narration.
- Skip emitting the final cursor position in a narration — the stop hook already provides the latest editor context, and it's slightly more up-to-date.

## 8.8 Stop hook exit code for "no narration pending"

- The stop hook currently returns a non-zero exit code (surfaced as "blocking error") when there's no pending narration. This is noisy — it's not an error, just "nothing to deliver."
- Distinguish cleanly: exit 0 with no output for "no narration pending", non-zero only for actual errors.

## 8.9 Listener restart instructions for transient failures

- Update the skill body (`claude_skill_body.md`) to instruct: "If the listener exits without producing a narration (empty output or non-zero exit code), restart it immediately — this is a transient failure, not a permanent error."

## 8.10 Research custom vocabulary / hotword list for transcription

- Crate names (serde, rubato, camino) and domain terms are not in speech models' training data, causing transcription errors ("Certie" for serde, "Roboto" for rubato).
- Research whether Parakeet or Whisper support hotword/vocabulary biasing.
- If not natively supported, consider a post-processing step that fuzzy-matches known technical terms from the project's dependency list or a user-configurable vocabulary file.

## 8.11 Research: agent-driven walkthrough via Zed ACP

- Investigate whether Zed's ACP (Agent Control Protocol) or extension API allows an external process to:
  - Open a file in the editor
  - Navigate to a specific line/selection
  - Scroll to reveal context
- If feasible, this enables a new workflow: agent prepares a walkthrough order (by data flow, dependency graph, or risk/complexity), opens each location in sequence, and the user narrates reactions
- Flips the dynamic from "user drives, agent follows" to "agent presents, user reacts" — agent as tour guide rather than stenographer
- This is research/exploration only — no implementation commitment in this phase

---

## Verification

- Manual test (8.1): delete model directory, run `/attend` in a Claude session -> see download progress, then narration works
- Manual test (8.2): record several narrations, set retention to 1 second, verify old archives are pruned after next receive
- 8.4: Snapshot tests for merge/render output include line ranges in elision markers
- Manual test (8.7): narrate while rapidly scanning through a file — verify fewer cursor-only blocks than before; verify cursor-only blocks that do appear have surrounding context lines
- Manual test (8.8): stop recording with no narration pending — verify no "blocking error" in agent output
- Manual test (8.9): kill listener process, verify agent restarts it without confusion
