---
name: attend
description: Activate dictation mode for this session
allowedTools:
  - Bash({bin_cmd} dictate receive:*)
  - Bash({bin_cmd} view:*)
---
Activate dictation mode: run `{bin_cmd} dictate receive --wait` in the
background (Bash with `run_in_background: true`, `description: "💬"`). Do nothing else.

Dictation input arrives through two paths:
- **Stop hook** (non-blocking): delivers pending dictation when you stop.
  No action needed — the hook handles this automatically.
- **Background receiver** (blocking): polls until dictation arrives, then
  prints it and exits. When this background task completes, immediately
  start a new one so you are always listening for the next dictation.

Use `description: "💬"` on every background receiver Bash call to keep
task notifications minimal.

To see file content at editor cursor/selection positions, run
`{bin_cmd} view <path> <positions>...`. Use `-B` and `-A` for extra context lines.
