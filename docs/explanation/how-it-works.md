# How it works

## The narration loop

When you press the narration hotkey, you set in motion a loop that converts
your spoken thoughts and on-screen actions into a single chronological document
that your coding agent can act on.

1. **A daemon starts (or resumes).** The CLI sends a command to the recording
   daemon. If no daemon is running, one is spawned as a detached background
   process with the transcription model loaded.

2. **Audio and context are captured in parallel.** The daemon opens your default
   microphone input and records 16 kHz audio, using a voice activity detector
   (VAD) to split the stream into segments on silence boundaries. At the same
   time, it periodically polls your editor for open files, cursor positions, and
   selections; monitors the clipboard for changes; listens for browser
   selections via a native messaging bridge; and receives shell commands from
   shell hooks.

3. **You press the hotkey again.** The daemon stops recording.

4. **Transcription runs locally.** Each audio segment is transcribed by a
   speech-to-text model running entirely on your machine. No audio leaves your
   computer. See [configuration](../reference/configuration.md#transcription-engines) for the
   available engines.

5. **Events are merged into a narrative.** Your transcribed words are
   interleaved chronologically with the captured context events — editor
   snapshots, file diffs, clipboard changes, browser selections, and shell
   commands — producing a single markdown document. See
   [the merge pipeline](#the-merge-pipeline) for how the merging works.

6. **The narrative is delivered.** The rendered narration is written to a
   pending file. The agent is then notified and receives it as a prompt, so it
   can respond to what you said in the context of what you were doing.

## The recording daemon

The daemon is a persistent background process, not a one-shot command. When you
stop recording, it doesn't exit — it flushes content and enters an idle state
with the transcription model still loaded. The next time you start recording,
it resumes instantly.

This matters because loading a speech-to-text model takes several seconds. If
the daemon exited after every recording, you'd wait for model loading every
time you pressed the hotkey. Instead, the model stays warm across short breaks.

The daemon auto-exits after an idle timeout (default 5 minutes, configurable
via [`daemon_idle_timeout`](../reference/configuration.md#fields)). This balances
responsiveness (the model is ready when you need it) against resource usage
(it doesn't run indefinitely).

Communication between the CLI and daemon is filesystem-based: the two
coordinate using atomically written files, which the other polls for. See
[architecture](architecture.md#filesystem-based-ipc) for the design rationale.

## Sessions and delivery

A **session** is one continuous conversation with a coding agent. When you
activate narration in your agent (`/attend` or `/attend:start`), it creates a
**listening file** that claims ownership of narration delivery for that
session. Narration goes only to the session that owns the listening file, or
to the clipboard if no session is active.

This ownership model exists because you might have multiple agent sessions
open. Without it, narration would arrive in whichever session happened to be
listening, which might not be the one you're working in. The explicit
activation step — running `/attend` in the session you want — makes delivery
deterministic.

**Displacement** is how you switch: activating narration in a different session
takes over the listening file. The previous session's background listener
detects the change and exits gracefully. You don't need to stop the old session
first.

Delivery itself happens through two paths working together. **Hook delivery**
interrupts the agent during its turn whenever narration arrives, ensuring it
responds before continuing. **Background listening** lets the agent be notified
when narration arrives *between* turns, so it can start a new response. See
[extending reference](../extending/reference.md#narration-delivery-paths) for the
mechanical details.

## Scope and redaction

All context is scoped to the agent's working directory (plus any paths in
[`include_dirs`](../reference/configuration.md#fields)). Editor snapshots, file diffs, and
shell commands from outside this scope are **redacted**: replaced with a `✂`
marker that tells the agent something was there but omits the content.

This prevents accidental disclosure of code from other projects. If you're
working across multiple repos, add the other repo to `include_dirs` in your
[configuration](../reference/configuration.md). Paths in delivered narration are always
relativized to the working directory.

Note that non-project-scoped capture sources — the system clipboard,
accessibility API, and browser extension — are not subject to this filtering,
since they capture user-initiated selections that may intentionally reference
external content.

## The merge pipeline

The merge pipeline is what turns a bag of timestamped events into a coherent
narrative. It does more than just sort by time:

1. **Chronological sort.** All events (words, snapshots, diffs, selections,
   commands) are sorted by timestamp.

2. **Progressive subsumption.** If you selected a small region of code, then
   expanded to a larger selection that contains it — without speaking in
   between — the earlier narrow selection is dropped. The later, broader one
   subsumes it. Without this, the narration would contain a series of
   increasingly large but overlapping code blocks, which is noisy and confusing.

3. **Run splitting.** Events are grouped into "runs" separated by speech.
   Within each run, redundant snapshots are collapsed and diffs are
   net-changed: intermediate states are removed, and only the final diff is
   shown. This means if you edited a function three times between two
   sentences, the agent sees one diff with the net result, not three
   incremental changes.

4. **Rendering.** Each event is rendered to markdown in sequence, producing
   the final narration document. See [narration format](../reference/narration-format.md)
   for the rendering format.

The result is that the agent receives a clean, chronological narrative: your
words as prose, with code and context blocks inserted where the corresponding
actions occurred, without the noise of intermediate states.
