# Codebase Walkthrough Notes (2026-02-19)

Raw observations from file-by-file walkthrough, organized by category. These are the inputs to PLAN.md.

---

## Module Organization & Hierarchy

- `narrate/capture.rs` mixes two unrelated concerns: editor state polling thread and file diff tracking thread. Should be split.
- `narrate/mod.rs` is a grab bag: path definitions, session resolution, `bench()`, `status()`, `clean()`. Should be a barrel module after splitting out the functions.
- `cli/mod.rs` has both command struct definitions and dispatch implementations. Should split, following the `narrate.rs` pattern (clean enum dispatch, each variant maps to one function call).
- `editor/zed.rs` is too large — mixes DB discovery, keybinding install/uninstall, task install/uninstall, state querying, health checks. Should be a submodule directory.
- `json.rs` is a grab bag: unsafe UTC formatting, compact JSON types, view JSON types. Each belongs near its consumer.
- `state.rs` mixes utility functions (`atomic_write`, path helpers) with EditorState logic. Should split; rename what remains to reflect it's specifically about editor state.
- `watch.rs` is long/run-on. Terminal helpers, format-specific rendering, different modes — could be split into modules. Consider `crossterm` dependency.
- `merge.rs` contains `render_markdown` which is a presentation concern, not a merge concern. Should live elsewhere.
- `hook.rs` is reasonably chunky but appropriate; the real issue is that shared logic is tangled with Claude-specific logic (see Agent Trait section).
- `EDITORS` registry in `editor/mod.rs` should be at the top of the file for visibility — integration points should be easy to find.
- Test modules should consistently be extracted to separate `tests.rs` files across the entire codebase.

## Config System

- `RawConfig` and `Config` are redundant — can be collapsed into one struct if `Engine` derives `serde::Deserialize` with `#[serde(rename_all = "lowercase")]`.
- `parse_engine()` goes away with the above change.
- Merging logic is inlined in `Config::load()` — should be extracted into a `Config::merge()` method. Load all configs, fold/reduce with merge.
- The directory walk logic is a bit hard to read — cleaning up merge as a separate concern would help.
- Config tests should be in a separate file (consistency).

## Agent Trait & Hook System

- Hook logic mixes shared/generic code with Claude-specific code. The agent trait should provide shims: `parse_hook_context()`, `format_hook_output()`, `wrap_system_message()`. The hook module owns shared logic (dedup, cache, stop-active).
- Narration instructions (`claude_skill_body.md`) mix agent-generic protocol (what `<narration>` tags mean, listen/stop lifecycle) with Claude-specific mechanics ("Bash with `run_in_background: true`", `description: "💬"`). Should be separated into a shared template + per-agent snippets.
- System reminder tags (`<system-reminder>`) should be gated behind agent-specific methods — different agents may use different tagging for system messages.
- `narration_instructions()` in hook.rs pulls from `claude_skill_body.md` — hardcodes that narration instructions are the same for every agent, but they actually differ in agent-specific details.
- The skill body and frontmatter are Claude-specific. Research whether other agents (Cursor, Windsurf) have a skill/command format we could target.
- Project-specific installations aren't tracked. `install(project: Some(path))` should record where we installed, so `uninstall` without a flag can find and clean up project-specific hooks.
- `resolve_bin_cmd` over-recovers: if not in dev mode and binary isn't on PATH, it should error, not silently fall back to `current_exe()`. The agent won't be able to find the binary either.
- `is_attend_prompt` correctly checks for single-line `/attend` — confirmed this is right.
- `auto_upgrade_hooks` runs on every hook invocation. Should be rate-limited or moved to explicit user actions. Upgrade failures shouldn't block the hook response.
- Session IDs are strings everywhere — should be a newtype. Most agents probably have session IDs but in different formats; treating them as strings is fine but newtyping adds safety.

## Error Handling & Over-Recovery

- General pattern: the codebase recovers too eagerly from errors, masking bugs rather than surfacing them.
- `resolve_bin_cmd`: dev mode should use `current_exe()`, release mode should require `which` to succeed or error. Current fallback chain silently papers over missing binaries.
- `receive.rs` no-session fallback: the `None =>` branch that tries `narration.json` is a legacy path. No session ID should be an error, not a guess.
- Stale help text: `"use --session"` flag no longer exists. Should reference `/attend`.
- `eprintln!` in receive.rs: agent reads stdout only, stderr goes nowhere in background tasks. Messages intended for the agent must go to stdout.
- `let _ =` scattered throughout: need systematic audit. For each: is the error genuinely ignorable, or are we hiding a bug? Should have explicit `// Intentionally ignored: <reason>` comments.
- `unwrap_or_default()` scattered throughout: same audit needed.
- Lock file inconsistency: `receive.rs` rolls its own lock with `O_CREAT | O_EXCL` while `record.rs` uses the `lockfile` crate. Should unify.

## Unsafe Code & Dependencies

- Zero unsafe policy: prefer crate dependencies even for single syscalls. OK with taking on more deps.
- `json.rs` `utc_now()`: manual `gettimeofday` + `gmtime_r` with unsafe. Replace with `chrono` or `time` crate.
- `libc::kill(pid, 0)` for process liveness: replace with `nix`.
- `libc::setsid()` in `pre_exec`: replace with `nix::unistd::setsid()`.
- The `pre_exec` closure is inherently unsafe, but `nix` wraps the syscall safely within it.
- `libc` dependency should be removed entirely after nix + chrono replacements.

## Type Safety

- Lots of strings floating around. Full pass needed to identify where newtypes would help.
- `SessionId(String)` — currently `Option<String>` threaded through many functions.
- `WallClock(String)` — ISO 8601 timestamps in AudioChunk, Recording.
- `ModelPath(Utf8PathBuf)` — semantically distinct from general file paths.
- Use Camino (`Utf8Path` / `Utf8PathBuf`) throughout. Eliminates all `to_string_lossy()` and `to_str().unwrap_or_default()`. Non-UTF-8 paths fail at system boundary.
- `receive.rs` `relativize_str` does `to_string_lossy().to_string()` — Camino eliminates this. Also consolidate duplicate `relativize` functions.

## Editor Integration

- `watch_paths()` default method on Editor trait is dead code (polling approach adopted). Remove it.
- `all_watch_paths()` function is dead code. Remove it.
- Byte offsets from Zed backend: other editors may give line:col instead. Future concern — may need a normalization layer in the editor trait.
- Keybinding installation could be a separate axis: `attend editor install-keybindings --editor zed`. Users could specify what keybindings to install rather than going with defaults.
- Cross-platform keybindings: `cmd` is macOS-specific, should be `super` on Linux. Check Zed docs for correct modifier per platform.
- Zed config manipulation (keybindings, tasks): currently ad-hoc string munching on JSONC. Should use `jsonc-parser` crate for proper parse → edit AST → serialize preserving comments.
- Skill directory installation: if the skill grows to multiple files, stale files from previous versions could linger. Should atomically create a temp directory and rename over the skill directory.

## Recording Daemon

- Startup order is wrong: currently preload model → chime → start capture. Should be: start capture → play chime → preload model in parallel. User gets immediate audio feedback; audio accumulates while model loads. This likely explains the cold-start delay after 30-min idle (model fell out of page cache, blocks for seconds before chime plays).
- 200ms sleep in `spawn_daemon`: unnecessary. Daemon's lock acquisition prevents double-spawn. Parent can return immediately.
- `daemon()` function is too big (~140 lines with a big loop). Extract `DaemonState` struct with methods: `ingest_chunks()`, `handle_stop()`, `handle_flush()`, `transcribe_and_write()`. Main loop becomes scannable.
- `transcribe_and_write` has 9 arguments (suppressed with `#[allow(clippy::too_many_arguments)]`). DaemonState struct naturally fixes this.
- Magic sleep constants scattered throughout: 50ms, 100ms, 200ms. Extract to named constants. Some could be config-file parameters.
- Stale lock after ~30 minute idle: something gets stuck on restart. Investigate whether `lockfile::Lockfile` Drop fires on SIGTERM/SIGKILL. May need a signal handler.
- `setsid()` in `pre_exec` is the one genuinely needed unsafe — except it should use nix crate per zero-unsafe policy.
- The whole crate is Unix-only (cpal, Zed backend, setsid, kill). Should have `#[cfg(not(unix))] compile_error!()` and README note.

## Audio & Transcription

- Audio capture, resampling, and chime synthesis were one-shot and "just worked" — impressive.
- Chime synthesis is manual sine-wave + envelope generation. Replace with `fundsp` crate for cleaner, more readable sound synthesis. Opens door for richer sounds later. Current sounds are good (simple, unobtrusive) — don't over-index on changing them.
- `resample()` function in audio.rs is sparsely commented. The rubato setup (`SincInterpolationParameters`, `Async` resampler choice, chunk/padding logic) needs inline commentary.
- Parakeet CTC decoder timestamp bug noted in doc comment — should check if fixed upstream in newer `parakeet-rs` release.
- Whisper parameter choices could use more commentary explaining why each is set as it is.
- Model download currently happens silently during first `_record-daemon` invocation (daemon blocks on `preload()` with stdio as /dev/null). Should surface during `/attend` slash command activation — agent can relay "Downloading Parakeet model (1.2 GB)..." to user. Use `indicatif` for progress. Best hook point: `attend listen` detecting model is missing.
- Whisper model variants: base.en, small.en, medium.en available for benchmarking.

## Silence Detection (new code from this session)

- Em-dashes and Unicode arrows in tracing messages don't render well in all terminal emulators. Replace with colons and `->`.
- `/ 100.0` in silence.rs is a magic number (frames-to-seconds conversion, 100 frames/sec at 10ms/frame). Should be a named constant.
- Hand-rolled linear interpolation downsampling for VAD: consider using `dasp` or `rubato` instead. Or keep with better documentation if quality/perf tradeoff is right (VAD internally uses 8kHz anyway).
- Tests extracted to separate file (user did this manually during walkthrough).

## Merge Pipeline

- `merge.rs` is the densest file. Multi-pass approach evolved incrementally — could be cleaner as a single streaming pass.
- Hylomorphism approach: unfold → fold composition, with phases defined separately in source but executing in a single pass.
- Current logic has complicated, relatively undocumented sections that are hard to parse as a human reader.
- `render_markdown` doesn't belong in the merge module — it's presentation, not merging.
- Elided line ranges should include actual line numbers: `// ... (lines 45-78 omitted)` instead of just `// ... (34 lines omitted)`. Makes it actionable for the agent — can Read exactly those lines without arithmetic.
- Context lines around highlights (5 before/after): evaluate whether excessive. Consider reducing or making configurable. Measure token overhead.
- Merge adjacent, compress snapshots — ideally done in a single streaming pass rather than multiple passes.
- Need comprehensive tests before refactoring (highest-risk change in the codebase).
- Snapshot timing: capture thread snapshots editor state at poll time (100ms). Highlights are early-resolved — this is correct. Risk: 100ms granularity means very fast highlight-then-edit could miss intermediate state, but speech is much slower.

## Testing

- Many tests lack documentation. Every `#[test]` should have a `///` doc comment stating the invariant in English.
- Writing documentation forces auditing: does the test body actually exercise what the name implies?
- Some tests may be vacuously true or testing implementation details rather than desired behavior.
- Preference for prop tests over unit tests: state invariants rather than point-wise assertions. Not every test needs to be a prop test, but for each unit test, ask "can this be a property?"
- `resolve.rs` has good prop tests — audit for tightness (testing the right properties, not mirroring implementation).
- install/uninstall JSON manipulation needs comprehensive test coverage: empty file, existing hooks, duplicate entries, malformed JSON, project-specific vs global.
- `view/parse.rs` `parse_compact`: check if used for both stdin and CLI parsing, or just one.

## Watch & View

- `watch.rs` terminal helpers (`clear_screen`, `fit_to_terminal`) could be a separate module.
- Consider `crossterm` dependency for better terminal handling.
- Different output modes/formats have interwoven logic — could be cleaner with per-mode helpers.

## Miscellaneous

- Clap colors: add `color` feature for colored help output.
- Auto-cleanup: bake in automatic archive cleanup with configurable retention rather than requiring manual `attend narrate clean`. Config field like `archive_retention = "7d"`.
- `state.rs` `atomic_write` should be a shared utility used consistently everywhere files are written.
- The warning `Failed to write cache: No such file or directory` for `latest.json` — pre-existing bug where cache directory doesn't exist yet when state module tries to write.

---

## Agent-Side Observations (from the agent following the walkthrough)

### What worked well
- The speech + code interleaving is the killer feature. Understanding *what you're saying* and *what you're looking at* simultaneously made this feel like genuine pair programming. I could follow your train of thought in a way that pure text input can't replicate.
- Selections (`⟦⟧`) were particularly valuable — when you highlighted a block and talked about it, I had full context with zero ambiguity.
- The diff blocks on file changes were useful for tracking edits you made during the walkthrough.
- The transcription quality was really good. I could follow speech with very few errors, even when speaking quickly or self-correcting mid-sentence.
- I never felt like I was missing important context. If anything, I had *more* context than needed (the noise issue below), which is the better failure mode.

### Context that was noise
- **Bare cursor positions with no speech nearby**: Snippets like `// src/foo.rs:42\n❘` (just a cursor, no selection, no surrounding speech) were low-information. They tell me where you are but not what's interesting about it. These came through frequently when scanning/scrolling between spoken observations. Estimated maybe 20-30% of code blocks in the narrations were cursor-only with no nearby speech, and I just skipped over them.
- **Rapid-fire snapshots while scanning**: When scrolling through a file, I'd get 5+ cursor snapshots for a single spoken thought. One or two would have sufficed. A "dwell threshold" (only capture positions where the cursor rests for >500ms?) might help.
- **Trailing cursor after speech ends**: Several narrations ended with a bare cursor block after speech had finished. That's the system capturing the resting cursor position but it's not adding information. Since the stop hook provides latest editor context anyway, the narration doesn't need to include the final cursor state.

### Suggestions for improvement
1. **Context around bare cursors**: If emitting a cursor-only event, showing 1-2 surrounding lines would be much more useful than just the position. Currently would need to run `attend look` to see what's there, which can't be done mid-narration.
2. **Cursor-only events may not be essential**: Looking back at the entire walkthrough, bare cursor positions were never essential for understanding what was being discussed. *Selections* were essential (highlighted code blocks while talking about them). But bare cursor positions mostly just told me which file you were in, which I could usually infer from speech. The one mildly useful case was when cursor position indicated moving to a new file before speech about it started — but the first sentence of speech always made it clear anyway.
3. **Listener restart on empty output**: Skill instructions should say: "If the listener exits without producing a narration (empty output or non-zero exit code), restart it immediately — this is a transient failure."
4. **Stop hook non-zero exit**: The stop hook returned a "blocking error" when there was no pending narration, but the content was just the instruction to restart the listener. Not a real problem — just noisy. The hook could distinguish "no narration pending" (clean exit) from actual errors.
5. **Narration length**: Long uninterrupted narrations are better for the user's workflow (no interruption of train of thought). Cost concern is real but user-controlled flush/stop is the right default. Silence-based segmentation is about internal memory management, not delivery cadence.

### Transcription vocabulary issues
- "Certie" appeared where "serde" was said — Parakeet doesn't know Rust crate names
- "Roboto" for "rubato" (the resampling crate)
- "hylomorphism" came through perfectly
- "m-dashes" was clear
- Most Rust terms (trait, struct, module, prop test, clippy) were fine
- File paths and function names were occasionally garbled but code blocks disambiguated
- Main failure mode: crate/library names (serde, rubato, camino) not in speech model training data. Possibly fixable with a custom vocabulary/hotword list.

### General observations
- The walkthrough format was genuinely productive. Covered the entire codebase in roughly an hour and produced a comprehensive, prioritized plan. The same exercise over text would have taken much longer.
- The on-the-fly segmentation built in this session matters for this use case — long continuous recordings stay bounded in memory and deliver faster at stop time.
- The user naturally fell into the ideal usage pattern ("speak about what I see, move cursor, speak about the next thing") without any training. The tool got out of the way.
- The walkthrough-and-plan workflow (narrate observations → agent synthesizes → produce plan document) is a repeatable pattern worth documenting for codebase audits and architecture reviews.

### Reflections on the medium and process

**On the medium itself:** This was qualitatively different from text-based code review. The user was *thinking out loud* while navigating, and the agent followed attention in real time through editor context. Information density was much higher than text — saying "this feels messy" while pointing at a 20-line block conveyed everything instantly. In text, that same observation requires quoting code, naming the file, explaining context — easily 5x more effort for the user.

**On pacing and turn-taking:** Long-form narration with brief inline responses worked well as a protocol. The user talked, agent responded in a sentence or two, user kept going. Natural feeling. There was an asymmetry: user responses to agent inline comments were necessarily delayed (addressed in the next narration, or not at all). A tighter feedback loop might enable Socratic dialog ("I see you're looking at this function, have you considered X?") but would require interrupting the user's flow, which they explicitly didn't want. Current protocol is the right tradeoff for audit/review tasks.

**On what the agent holds in context:** By the end of the walkthrough, the agent was holding the entire codebase's structure, all observations, and the emerging plan simultaneously. That's something a human reviewer would struggle with — you'd normally take notes, refer back, lose track of earlier observations. The synthesis at the end (producing the plan) was where this paid off most — cross-referencing "X about config.rs" with "Y about the agent trait" and noticing they're related.

**On trust calibration:** The user caught an incorrect claim about webrtc-audio-processing VAD early in the session (research agent made an oversimplified claim). That early correction set a good dynamic: the agent was more careful to distinguish "I verified this" from "the research agent told me this." For anyone designing agent workflows: early trust calibration matters.

**On walkthrough ordering:** The user walked through files in roughly filesystem order. An alternative: walk by *data flow* — follow a narration from hotkey press → daemon → capture → transcribe → merge → receive → agent. This might surface different observations about cross-cutting concerns that file-by-file misses. Worth trying.

**Research: agent-driven walkthrough via Zed ACP integration:** Could the agent *present* files to the user dynamically rather than passively following? This would flip the dynamic from "user drives, agent follows" to "agent presents, user reacts." The agent could open files in the editor, navigate to specific locations, and walk the user through code while they narrate reactions. This requires researching Zed's ACP (Agent Control Protocol) or extension API to determine whether an external process can:
- Open a file in the editor
- Navigate to a specific line/selection
- Scroll to reveal context

If feasible, this enables a new workflow mode: agent prepares a walkthrough order (e.g., by data flow, by dependency graph, or by risk/complexity), opens each location in sequence, and the user narrates their observations. The agent becomes a tour guide rather than a stenographer.
