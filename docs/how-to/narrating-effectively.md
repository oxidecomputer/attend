# How to narrate effectively

You've set up `attend` and completed your first narration. This guide covers
how to get the most out of narrating as a way of working with your coding
agent.

## Speak naturally

The most important thing is to talk the way you'd talk to a colleague sitting
next to you. Don't try to be precise or formal — the agent handles ambiguity
well, and natural speech is easier for the transcription model too. Say "this
function is wrong" while selecting it, not "the function `parse_config` at
line 42 of `src/config.rs` has a bug."

## Look at what you're talking about

The agent sees where your cursor is and what you have selected. If you say
"this should return an error instead," make sure your cursor is on the code
you mean. Navigate as you speak — move to a file, place your cursor, select a
region. The agent receives your words and actions interleaved chronologically,
so it understands "this" and "here" from context.

You don't need to select precisely. A rough selection or just having your
cursor in the right function is usually enough.

## Narrate across activities

Narration captures more than just your editor. While recording, you can:

- **Select text in your browser** to show the agent documentation, error
  messages, or API references you're reading.
- **Run commands in your terminal** — the agent sees what you ran, whether it
  succeeded, and how long it took (though not the output itself). If you want
  the agent to see output too, select it in the terminal or copy it.
- **Copy text or images** to the clipboard — the agent sees those too.

All of these are woven into the narration alongside your speech. So you can
say, "I'm looking at the docs for this API" while selecting the relevant
passage in Firefox, and the agent receives both your words and the selected
text.

## Keep narrations focused

A narration works best when it's about one thing: a single task, question, or
observation. If you find yourself switching topics, stop and deliver, then
start a new narration for the next thought. Short, focused narrations give the
agent a clear task to respond to.

If you want to narrate continuously without stopping the recording, use
`attend narrate start` (or the `⌘ :` hotkey in Zed) instead of toggle. This
delivers the current narration and keeps recording, so you can work in a
stream of short deliveries.

## Think out loud

Narration is at its most useful when you share your reasoning, not just your
conclusions. Instead of "refactor this function," try narrating your thought
process: "This function does too many things — it parses the input, validates
it, and writes to the database. I think we should split the validation out
into its own function so we can test it independently."

The agent gets more context about your intent, which means it makes better
decisions about how to carry out the work.

## Walk the agent through code

Narration isn't only for giving instructions. It's also a good way to build
the agent's understanding of a codebase by walking through it: open files,
point at structures, explain how they relate. "This is the main entry point.
It calls into this module here, which handles all the IPC. The message types
are defined over here..." The agent retains that context for the rest of the
conversation.

## Redirect the agent mid-task

If the agent is working and you realize it's headed in the wrong direction,
narrate a correction. Narration is delivered between tool calls, so the agent
will receive it before it takes its next action. "Actually, don't change that
file — the real problem is in the caller" is enough to redirect.

## Pause to go off the record

Use `attend narrate pause` (or the `⌘ {` hotkey in Zed) when you need to do
something that shouldn't end up in the narration — have a conversation with
someone in the room, work on something unrelated in your editor, or step away
for a moment. Pausing suspends all capture (audio, editor, clipboard,
everything). When you resume, the narration continues as one piece.

You don't need to pause for silence. If you stop talking to read or think,
that's fine — silence is trimmed automatically, and the agent never sees it.

## Preview with yank

If you're curious what the agent receives, or want to review before sending,
use `attend narrate yank` (or the `⌘ }` hotkey in Zed). This copies the
rendered narration to your clipboard instead of delivering it. You can paste
it somewhere to inspect the result.
