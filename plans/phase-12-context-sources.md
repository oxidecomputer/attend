# Phase 12: External Context Sources

Capture highlighted text, clipboard contents, and browser context during
narration sessions. The goal: when narrating while looking at Firefox, iTerm2,
or any other app, the agent receives what you're pointing at, tagged with where
it came from.

## Motivation

attend currently captures three event streams during narration: speech (Words),
editor state (EditorSnapshot), and file changes (FileDiff). These work great
when the user is in Zed, but developers spend significant time outside their
editor: reading docs in Firefox, looking at terminal output in iTerm2, reviewing
PRs, reading error messages. Today that context is invisible to the agent; the
user has to describe it verbally or copy-paste.

This phase adds two new capture streams:

1. **macOS Accessibility**: selected text from *any* focused app, tagged with
   the app name and window title
2. **Firefox native messaging**: selected text from Firefox specifically, tagged
   with the page URL, title, and whether the selection is code

Together these cover the "Zed + Firefox + iTerm2 triple-pane" workflow: Zed is
handled by EditorSnapshot, Firefox by the browser extension (with Accessibility
as fallback), and iTerm2 by Accessibility.

## Design overview

### New Event variants

```rust
enum Event {
    // ... existing variants ...

    /// Text selected in an external application (via macOS Accessibility API).
    ExternalSelection {
        offset_secs: f64,
        /// Application name (e.g., "Firefox", "iTerm2", "Safari").
        app: String,
        /// Window title (e.g., page title, terminal tab name).
        window_title: String,
        /// The selected text.
        text: String,
    },

    /// Text selected in a browser, with rich page context.
    /// Delivered via Firefox/Chrome native messaging extension.
    BrowserSelection {
        offset_secs: f64,
        /// Page URL.
        url: String,
        /// Page title.
        title: String,
        /// The selected text.
        text: String,
        /// Whether the selection is inside a <code>/<pre> block.
        is_code: bool,
    },
}
```

BrowserSelection takes priority over ExternalSelection when both fire for the
same text (the browser extension provides richer context). The dedup logic is:
if an ExternalSelection and BrowserSelection arrive within ~500ms with the same
text content, drop the ExternalSelection.

### Markdown rendering

ExternalSelection:
```
> [iTerm2: ~/src/attend] "error[E0308]: mismatched types"
```

BrowserSelection:
````
> [docs.rs/tokio - tokio::process](https://docs.rs/tokio/latest/tokio/process/)
```rust
pub fn spawn(&mut self) -> Result<Child>
```
````

Non-code browser selections render as blockquotes; code selections render as
fenced code blocks with language detection from the page context.

### Capture architecture

```
                    +-----------------+
                    | Recording Daemon |
                    | (attend narrate) |
                    +--------+--------+
                             |
              +--------------+--------------+
              |              |              |
     speech capture   editor capture   [NEW] external capture
     (audio thread)   (editor_capture)  (ax_capture thread)
                                           |
                                    Swift helper binary
                                    (attend-ax-helper)
                                           |
                              +------------+------------+
                              |                         |
                     AXSelectedText              frontmost app +
                     from focused element        window title

    [SEPARATE PROCESS — runs independently of recording]

    Firefox extension
         |
    native messaging (stdin/stdout JSON)
         |
    attend browser-bridge
         |
    writes BrowserSelection events to pending dir
```

The Accessibility capture runs inside the recording daemon (like editor_capture).
The browser bridge runs as a separate process (launched by Firefox on demand).

---

## Part A: macOS Accessibility via Swift helper

### A1. Swift helper binary (`attend-ax-helper`)

A minimal Swift CLI (~60 lines) that:
1. Calls `AXUIElementCreateSystemWide()`
2. Queries `kAXFocusedUIElementAttribute` on the system-wide element
3. Queries `kAXSelectedTextAttribute` on the focused element
4. Queries `kAXFocusedWindow` + `kAXTitleAttribute` for window title
5. Gets `NSWorkspace.shared.frontmostApplication` for app name
6. Prints JSON to stdout:
   ```json
   {"app": "Firefox", "window_title": "tokio::process - Rust", "selected_text": "pub fn spawn(...)"}
   ```
   Prints `{"app": "...", "window_title": "...", "selected_text": null}` when
   nothing is selected.

Build: `swiftc -O attend-ax-helper.swift -o attend-ax-helper`. No runtime
dependencies. Output is a single static binary.

**Build integration options** (decide during implementation):
- **build.rs**: Compile with `swiftc` if available, skip if not (graceful
  degradation on Linux or CI without Swift toolchain)
- **Makefile/justfile target**: Explicit `just build-ax-helper`
- **Pre-built**: Check in the compiled binary (simplest, but platform-specific)

Recommended: build.rs with graceful skip. The feature is macOS-only anyway.

**Latency budget**: 15-40ms per invocation (fork+exec+AX query). At 200ms
polling interval, this consumes ~10-20% of the budget. Acceptable.

### A2. Accessibility capture thread (`ax_capture.rs`)

New module `src/narrate/ax_capture.rs`, structured like `editor_capture.rs`:

- Spawns a thread that polls every 200ms
- Shells out to `attend-ax-helper` and parses the JSON response
- Emits `Event::ExternalSelection` when selected text changes
- Dwell logic: only emit after selection stabilizes for ~300ms (same concept as
  cursor dwell in editor_capture)
- Dedup: suppress if selected text is identical to the previous emission

**Filtering**:
- Skip emissions when the frontmost app is the editor (Zed) — that's already
  covered by EditorSnapshot
- Skip emissions when the frontmost app is the terminal running Claude Code —
  the agent sees its own output already
- Config: `ax_capture_ignore_apps = ["Zed", "Code"]` in config.toml (optional,
  sensible defaults)

**Integration into CaptureHandle**:
- Add a third thread alongside `editor_thread` and `diff_thread`
- New `ax_events: Arc<Mutex<Vec<Event>>>` in CaptureHandle
- `drain()` and `collect()` return all three event streams

### A3. Permissions UX

The Accessibility permission is granted to the *terminal emulator* (iTerm2),
not to attend itself, because macOS TCC traces the "responsible process" up
the process tree. Most developers using accessibility-aware tools have already
granted this.

If the permission is missing, the Swift helper will return an error or empty
results. attend should:
1. Detect the "not trusted" state (AXIsProcessTrusted() returns false)
2. On first recording start, print a one-time message:
   "External text capture requires Accessibility permission for your terminal.
   Grant it in System Settings > Privacy & Security > Accessibility."
3. Gracefully degrade: ax_capture thread skips polling, no events emitted

### A4. Per-app reliability notes

| App | AXSelectedText | Notes |
|-----|---------------|-------|
| Safari | Works | |
| Chrome | Works | May need `--force-renderer-accessibility` |
| iTerm2 | Works | |
| VS Code | Works | |
| Slack | Partial | Electron; line-break bugs |
| Firefox | Unreliable | Historically broken for web content (Bug 674612). Part B (browser extension) is the fix. |

The Firefox gap is the primary motivation for Part B.

---

## Part B: Firefox native messaging extension

### B1. Extension structure

```
extension/
  manifest.json        MV3 manifest
  background.js        Relays messages to native app
  content.js           Observes selections, gathers DOM context
```

**manifest.json** (Manifest V3, Firefox):
```json
{
  "manifest_version": 3,
  "name": "Attend Browser Bridge",
  "version": "1.0",
  "browser_specific_settings": {
    "gecko": {
      "id": "attend@oxide.computer",
      "strict_min_version": "109.0"
    }
  },
  "permissions": ["nativeMessaging"],
  "host_permissions": ["<all_urls>"],
  "content_scripts": [{
    "matches": ["<all_urls>"],
    "js": ["content.js"]
  }],
  "background": {
    "scripts": ["background.js"]
  }
}
```

**content.js** (~40 lines): Listens for `selectionchange` (debounced 300ms).
On stable selection, gathers:
- `window.getSelection().toString()` — the text
- `location.href` — the URL
- `document.title` — the page title
- `element.closest('code, pre, .highlight')` — is it code?
- Sends to background script via `browser.runtime.sendMessage()`

**background.js** (~15 lines): Receives messages from content script, relays
to native app via `browser.runtime.sendNativeMessage("attend", message)`.
Uses `sendNativeMessage` (one-shot), not `connectNative` (persistent), because
MV3 non-persistent backgrounds kill persistent connections on idle.

### B2. Native messaging host

**`attend browser-bridge` subcommand**: Reads one length-prefixed JSON message
from stdin, processes it, writes one length-prefixed JSON response to stdout,
exits. Stateless, one-shot.

The message contains:
```json
{
  "type": "selection",
  "text": "pub fn spawn(&mut self) -> Result<Child>",
  "url": "https://docs.rs/tokio/latest/tokio/process/",
  "title": "tokio::process - Rust",
  "is_code": true
}
```

Processing:
1. Parse the JSON message
2. Write a `BrowserSelection` event to the narration pending directory
   (as a JSON file, same format as other narration events)
3. Respond with `{"status": "ok"}` and exit

**Deciding where to write**: The browser bridge doesn't know which session is
active. It writes to a well-known location
(`<cache_dir>/browser-pending/<timestamp>.json`). The recording daemon (or
the receive pipeline) picks these up and merges them into the event stream.

Alternative: Write directly to the active session's pending directory by
reading the `listening` file to determine the session ID. This is simpler
and avoids a new directory, but creates a coupling between the browser bridge
and the session state.

Recommended: Read the `listening` file. The coupling is acceptable because the
browser bridge is part of attend, not an external tool.

**Protocol framing**: The `native_messaging` crate handles the 32-bit
length-prefix stdin/stdout protocol for both Firefox and Chrome. Alternatively,
hand-roll it (~20 lines: read 4 bytes, parse as u32 LE, read that many bytes,
parse as JSON; reverse for write). The protocol is simple enough that a
dependency may not be worth it.

### B3. Native messaging host manifest

**Location** (macOS, per-user):
```
~/Library/Application Support/Mozilla/NativeMessagingHosts/attend.json
```

**Contents**:
```json
{
  "name": "attend",
  "description": "Attend browser bridge",
  "path": "/Users/<user>/.cargo/bin/attend",
  "type": "stdio",
  "allowed_extensions": ["attend@oxide.computer"]
}
```

`attend install --browser firefox` writes this file, using the resolved binary
path (same pattern as `attend install --agent claude`).

### B4. Extension distribution

**For development**: `web-ext run` or `about:debugging` > Load Temporary Add-on.

**For release**: Submit to AMO (addons.mozilla.org) as an **unlisted** extension.
Signing is free and fast. Users install from a direct `.xpi` URL that attend
can print during `attend install --browser firefox`.

**For Firefox Developer Edition / Nightly**: Can load unsigned via
`xpinstall.signatures.required = false`.

### B5. Chrome compatibility (future)

The content script and background script are nearly identical. Differences:
- Chrome MV3 uses `background.service_worker` instead of `background.scripts`
- Chrome uses `allowed_origins` in the native manifest instead of
  `allowed_extensions`
- Chrome native manifest location:
  `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`
- Chrome uses `chrome.*` namespace (vs Firefox `browser.*`); the
  `webextension-polyfill` normalizes this

A build step that produces two `manifest.json` variants from one source tree
would cover both browsers. Not in scope for this phase but architecturally
trivial to add later.

---

## Part C: Integration into the narration pipeline

### C1. Merge and compression

`compress_and_merge()` in `merge.rs` needs new logic for ExternalSelection
and BrowserSelection events:

- **Chronological merge**: Both new types have `offset_secs` and sort naturally
  with existing events
- **Dedup**: When an ExternalSelection and BrowserSelection arrive within 500ms
  with matching text, keep only the BrowserSelection (richer context)
- **Compression**: Consecutive ExternalSelection events from the same app with
  the same text → keep only the last one (same dwell logic as cursor snapshots)

### C2. Rendering in `render.rs`

ExternalSelection renders as a blockquote with source annotation:
```markdown
> [iTerm2: ~/src/attend] "error[E0308]: mismatched types"
```

BrowserSelection renders as a blockquote with linked source. Code selections
get a fenced code block:
```markdown
> [tokio::process - Rust](https://docs.rs/tokio/latest/tokio/process/)
```rust
pub fn spawn(&mut self) -> Result<Child>
```
```

Non-code browser selections:
```markdown
> [tokio::process - Rust](https://docs.rs/tokio/latest/tokio/process/)
> "Spawns the command as a child process, returning a handle to it."
```

### C3. Filtering in `receive.rs`

ExternalSelection and BrowserSelection events are **not** file-path-based, so
the existing `filter_events` / `path_included` logic doesn't apply. These
events pass through the filter unconditionally (they don't contain project
file paths, so there's nothing to leak).

The `relativize_events` pass is a no-op for these types.

### C4. Snip config

Large selections (e.g., user highlights a whole page) should be truncated.
Apply `SnipConfig` to `ExternalSelection.text` and `BrowserSelection.text`
the same way it's applied to EditorSnapshot content.

---

## Part D: Install / uninstall

### `attend install --browser firefox`

1. Write the native messaging host manifest to
   `~/Library/Application Support/Mozilla/NativeMessagingHosts/attend.json`
2. Print instructions for installing the browser extension:
   - Development: "Load from `<attend-source>/extension/` via about:debugging"
   - Release: "Install from <AMO URL>"
3. Record the installation in `InstallMeta` for `attend uninstall` cleanup

### `attend uninstall --browser firefox`

1. Remove the native messaging host manifest
2. Print reminder to remove the browser extension from Firefox

### No install step for Accessibility

The Swift helper is compiled alongside attend (or shipped as a companion
binary). No user-facing install step. The permission prompt is handled by
macOS automatically on first use.

---

## Task breakdown

### Phase 12a: Accessibility capture (Part A)

| # | Task | Depends on |
|---|------|-----------|
| A1 | Write `attend-ax-helper.swift`, test manually | — |
| A2 | build.rs integration: compile Swift helper if `swiftc` available | A1 |
| A3 | New `Event::ExternalSelection` variant + serde | — |
| A4 | `ax_capture.rs`: polling thread, dwell, dedup | A1, A3 |
| A5 | Wire ax_capture into `CaptureHandle` (third thread) | A4 |
| A6 | `render.rs`: render ExternalSelection as blockquote | A3 |
| A7 | `merge.rs`: compress consecutive same-app selections | A3 |
| A8 | `receive.rs`: pass ExternalSelection through filter unchanged | A3 |
| A9 | Graceful degradation when AX permission is missing | A4 |
| A10 | Config: `ax_capture_ignore_apps` | A4 |
| A11 | Tests: prop tests for merge/compress with mixed event types | A7 |

### Phase 12b: Firefox native messaging (Part B)

| # | Task | Depends on |
|---|------|-----------|
| B1 | New `Event::BrowserSelection` variant + serde | — |
| B2 | Write `content.js` + `background.js` + `manifest.json` | — |
| B3 | `attend browser-bridge` subcommand (native messaging protocol) | B1 |
| B4 | Write BrowserSelection events to session pending dir | B3 |
| B5 | `attend install --browser firefox` (native host manifest) | B3 |
| B6 | `render.rs`: render BrowserSelection (code vs prose) | B1 |
| B7 | `merge.rs`: dedup ExternalSelection vs BrowserSelection | B1, A3 |
| B8 | `receive.rs`: pass BrowserSelection through filter unchanged | B1 |
| B9 | `attend uninstall --browser firefox` | B5 |
| B10 | Tests: native messaging protocol round-trip | B3 |
| B11 | AMO submission for extension signing | B2 |

### Dependencies between 12a and 12b

12a and 12b are **independent** — they can be built in either order or in
parallel. The only shared task is B7 (dedup between the two event types),
which requires both A3 and B1.

---

## Future work (not in this phase)

These ideas came out of the brainstorm and are worth noting but not planned:

### Clipboard monitoring (`clipboard-rs`)
Poll `NSPasteboard.changeCount`, read text + source URL (Chrome:
`org.chromium.source-url`), tag with frontmost app. Complements Accessibility
(captures explicit copy actions vs passive selections). New
`Event::ClipboardCapture`. Note: macOS 16 will prompt for clipboard read
permission.

### Command output capture (`attend wrap`)
PTY passthrough via `portable-pty` crate. User runs `attend wrap cargo test`;
attend shows output normally while capturing it. New `Event::CommandOutput`.
Shell abbreviations for zero-friction UX. Strip ANSI for narration storage.

### Fish shell hooks
`fish_preexec`/`fish_postexec` for lightweight metadata: command text + exit
code + duration. No output capture, but "user ran cargo test and it failed"
is useful context even without the output.

### iTerm2 Python API (Rust protobuf client)
Richest terminal integration: screen contents, selection, command lifecycle
via `PromptMonitor`. Protobuf over Unix socket. A Go client (`tmc/it2`)
proves the protocol is language-agnostic. High effort, iTerm2-only.

### Chrome extension
Same content/background scripts as Firefox with a second manifest. The
`webextension-polyfill` normalizes the API. Separate native messaging host
manifest path.

### Safari extension
Requires a full native macOS app wrapper (XPC, not stdin/stdout). Much
larger effort. Not practical without significant demand.
