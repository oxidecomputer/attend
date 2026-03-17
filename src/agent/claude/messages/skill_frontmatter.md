---
name: {skill_name}
description: Activate narration mode for this session. Use when the user asks you to start listening, attend, or enable voice narration.
allowedTools:
  - Bash({bin_cmd} listen:*)
  - Bash({bin_cmd} look:*)
  - Bash({bin_cmd} glance:*)
  - Read(~/Library/Caches/attend/narration/staging/clipboard/**)
  - Read(~/.cache/attend/narration/staging/clipboard/**)
---
