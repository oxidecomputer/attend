# How it works

## Data flow

When you press the narration hotkey, the following happens:

1. **Daemon starts (or resumes).** The CLI sends a command to the recording
   daemon. If no daemon is running, one is spawned as a detached background
   process with the transcription model loaded.

2. **Audio capture begins.** The daemon opens your default microphone input
   and records 16 kHz audio. A voice activity detector (VAD) splits the
   audio stream into segments on silence boundaries.

3. **Context is captured in parallel.** While recording, the daemon
   periodically polls your editor for open files, cursor positions, and
   selections. It also monitors the clipboard for changes, listens for
   browser selections via a native messaging bridge, and receives shell
   commands from shell hooks.

4. **You press the hotkey again.** The daemon stops recording.

5. **Transcription runs locally.** Each audio segment is transcribed to text
   by a local speech-to-text model (Parakeet or Whisper). No audio leaves
   your machine.

6. **Events are merged chronologically.** Your transcribed words are
   interleaved with the context events (editor snapshots, file diffs,
   clipboard, browser selections, shell commands) based on their timestamps,
   producing a single chronological narrative.

7. **Rendering.** The merged events are rendered as markdown: prose for your
   speech, fenced code blocks for editor snapshots, diff blocks for file
   changes, blockquotes for selections, and shell-fenced blocks for
   commands.

8. **Delivery.** The rendered narration is written to a pending file. The
   agent is notified that new narration has arrived either through its hook
   system (at the next tool boundary) or via a background listener that delivers
   it as a new conversational turn. The agent is prompted (and constrained by
   hooks) to relaunch the background listener, the act of which *only then*
   delivers the narration to the agent, ensuring the conversation can continue.

## The recording daemon

The daemon is a **persistent background process**. When you stop recording, it
doesn't exit; it flushes content and enters an idle state with the transcription
model still loaded. The next time you start recording, it resumes instantly
without reloading the model or spawning a new process.

The daemon auto-exits after an idle timeout (default 5 minutes, configurable via
`daemon_idle_timeout`). This means the model stays warm across short breaks but
doesn't consume resources indefinitely.

Communication between the CLI and daemon is **filesystem-based**: the two
coordinate using atomically written files, which the other polls for. (This
could be improved for even lower latency in the future.)

## Session model

A **session** is one continuous conversation with a coding agent (e.g., one
Claude Code session). Each session has a unique ID.

When you activate narration in your agent (`/attend` or `/attend:start`), it
creates a **listening file** that claims ownership of narration delivery for
that session. Narration is delivered only to the session that owns the listening
file, or to the clipboard if no session exists.

**Displacement:** if you activate narration in a different session, it takes
over the listening file. The previous session's background listener detects the
change and exits gracefully. This means you can freely switch which session
receives your narration.

## Hook lifecycle

The agent integration works through **hooks** — lifecycle events that the
agent fires at specific points:

| Hook | When it fires | What attend does |
|------|---------------|------------------|
| `SessionStart` | New conversation begins | Clear stale state, auto-upgrade if binary version changed, emit initial instructions |
| `UserPromptSubmit` | User sends a message | Inject editor context (what files and cursors are visible) |
| `PreToolUse` | Before a tool runs | Notify about any pending narration |
| `PostToolUse` | After a tool runs | Notify about any pending narration |
| `Stop` | Agent turn ends | Notify about any pending narration |

The PreToolUse and PostToolUse hooks ensure narration arrives **between tools**
within a single agent response, not just at turn boundaries. They do this by
forcing the agent to re-run `attend listen` to both (1) receive the narration,
and (2) restart the background listener to ensure future narration can be
delivered.

## Scope and redaction

All context is scoped to the agent's working directory (plus any paths in
`include_dirs`). Editor snapshots, file diffs, and shell commands from outside
this scope are **redacted**: replaced with a `✂` marker that tells the agent
something was there but omits the content. This prevents accidental disclosure
of code from other projects.

Paths in delivered narration are always **relativized** to the working directory.

## Local transcription

Two speech-to-text engines are available, both running entirely on your machine:

| Engine | Model | Notes |
|--------|-------|-------|
| **Parakeet** (default) | [Parakeet TDT 0.6B](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx) (ONNX) | Better quality, multilingual, faster |
| **Whisper** | [Whisper Small](https://huggingface.co/ggerganov/whisper.cpp) (GGML) | Smaller download, English-focused |

The model is downloaded automatically on first use and cached locally. You can
switch engines or provide a custom model path in the
[configuration](setup.md#configuration).

## The merge pipeline

The merge pipeline turns raw timestamped events into a coherent narrative:

1. **Chronological sort.** All events (words, snapshots, diffs, selections,
   commands) are sorted by their timestamp.

2. **Progressive subsumption.** If you selected a small region of code, then
   expanded to a larger selection that contains it (without speaking any words
   in between), the earlier narrow selection is dropped — the later, broader one
   is more informative.

3. **Run splitting.** Events are grouped into "runs" separated by speech.
   Within each run, redundant snapshots are collapsed and diffs are
   net-changed (intermediate states are removed, only the final diff is
   shown).

4. **Rendering.** Each event is rendered to markdown in sequence.

The result is a chronological narrative: your words appear as prose, with code
and context blocks inserted where the corresponding events occurred.
