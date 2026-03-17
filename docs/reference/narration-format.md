# Narration format reference

This page describes the format of rendered narration — the markdown document
produced when you stop recording, or visible when you [yank to
clipboard](commands.md#attend-narrate-yank). See [example
narration](../tutorial/example-narration.md) for a realistic sample.

## Event types

Narration interleaves the user's speech with structured context events, all
ordered chronologically.

| Type | Marker | Source |
|------|--------|--------|
| [Prose](#prose) | Plain text | Transcribed speech |
| [Editor snapshots](#editor-snapshots) | `` `path:line`: `` + code fence | Cursor/selection in editor |
| [File diffs](#file-diffs) | `` `path`: `` + `diff` fence | Edits made during narration |
| [Shell commands](#shell-commands) | `$ ` prefix in shell fence | Commands run in terminal |
| [Browser selections](#browser-selections) | Link attribution + blockquote | Text selected in browser |
| [External selections](#external-selections) | App attribution + blockquote | Text selected in other apps |
| [Clipboard](#clipboard) | Plain blockquote or `![clipboard]` | Copied text or images |
| [Redaction markers](#redaction-markers) | `✂` prefix | Filtered out-of-scope context |

### Prose

Plain text with no markers. The user's transcribed speech.

### Editor snapshots

A `` `path:line`: `` label above a fenced code block. The code block uses the
file's language for syntax highlighting:

`src/main.rs:42`:
```rust
fn main() {}
```

### File diffs

A `` `path`: `` label above a `diff`-fenced code block:

`src/lib.rs`:
```diff
-    pub timeout: u64,
+    pub timeout: Duration,
```

### Shell commands

A shell-tagged fence with `$ ` prefix. An optional `In <dir>/:` label appears
when the command ran outside the project root. A trailing `# exit <code>,
<dur>s` comment appears on failure or slow runs (absence means exit 0, fast):

In `subdir/`:
```fish
$ cargo test --lib  # exit 1, 3.2s
```

### Browser selections

A markdown link attribution above a blockquote:

[Rust docs](https://doc.rust-lang.org/std/):
> Returns the number of elements in the vector.

### External selections

An app/path attribution above a blockquote. These come from text selected in
non-browser applications (captured via macOS Accessibility):

iTerm2: ~/src/attend:
> error[E0308]: mismatched types

### Clipboard

Text appears as a plain blockquote with no attribution:

> fn merge(a: Config, b: Config) -> Config {

Images appear as `![clipboard](/path/to/image.png)`.

Clipboard text that duplicates a richer source (external or browser selection)
is automatically dropped.

### Redaction markers

A `✂` prefix with counts (e.g., `✂ 2 files, command`). Context from outside
the project directory was captured but filtered. Labels:

| Label | Source |
|-------|--------|
| `file` / `files` | Editor snapshots |
| `edit` / `edits` | File diffs |
| `command` / `commands` | Shell commands |
