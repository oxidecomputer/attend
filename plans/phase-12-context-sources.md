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
     (audio thread)   (editor_capture)  (ext_capture thread)
                                           |
                                    ExternalSource trait
                                           |
                              +------------+------------+
                              |            |            |
                           macOS        (Linux)      (future)
                        accessibility   AT-SPI      platforms
                           crate        (future)
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

The external capture thread is OS-agnostic: it polls an `ExternalSource` trait
for the current selection state. Platform backends implement that trait. Only
the macOS backend is implemented now; Linux (AT-SPI/D-Bus) and others can be
added later without touching the polling/dwell/dedup logic.

The browser bridge runs as a separate process (launched by Firefox on demand).

---

## Part A: External selection capture (platform-abstracted)

### A1. `ExternalSource` trait and module layout

The capture logic is split into a platform-agnostic polling/dwell/dedup
layer and platform-specific query backends:

```
src/narrate/
  ext_capture.rs          — ExternalSource trait, ExternalSnapshot,
                            DwellTracker, spawn() polling thread
  ext_capture/
    macos.rs              — macOS backend (accessibility crate)
```

**`ExternalSource` trait** (in `ext_capture.rs`):

```rust
/// A snapshot of the currently selected text in the focused application.
pub struct ExternalSnapshot {
    /// Application name (e.g. "Safari", "iTerm2").
    pub app: String,
    /// Window title (e.g. page title, terminal tab name).
    pub window_title: String,
    /// The selected text, if any.
    pub selected_text: Option<String>,
}

/// Platform-specific backend for querying external application state.
pub trait ExternalSource: Send {
    /// Check whether the platform's accessibility permission is granted.
    /// Returns `false` if queries will fail due to missing permissions.
    fn is_available(&self) -> bool;

    /// Query the current state of the focused application.
    /// Returns `None` if the query fails or no application is focused.
    fn query(&self) -> Option<ExternalSnapshot>;
}
```

**Platform dispatch** (in `ext_capture.rs`):

```rust
/// Construct the platform-appropriate ExternalSource, or None if the
/// current platform has no backend.
pub fn platform_source() -> Option<Box<dyn ExternalSource>> {
    #[cfg(target_os = "macos")]
    { Some(Box::new(macos::MacOsSource::new())) }

    #[cfg(not(target_os = "macos"))]
    { None }
}
```

When `platform_source()` returns `None`, the capture thread is not spawned.
No feature flags needed — cfg selects at compile time.

This design means a future Linux backend only needs to:
1. Add `src/narrate/ext_capture/linux.rs` implementing `ExternalSource`
2. Add a `#[cfg(target_os = "linux")]` arm to `platform_source()`
3. Add the platform-specific dependency gated on `cfg(target_os = "linux")`

No changes to the polling thread, dwell tracker, event types, merge pipeline,
renderer, or config.

### A2. macOS backend (`ext_capture/macos.rs`)

Uses the [`accessibility`](https://crates.io/crates/accessibility) crate
(v0.2.0, by eiz) to call the macOS AX API directly from Rust. This replaces
the originally-planned Swift helper binary, eliminating `swiftc` as a build
dependency, fork+exec overhead per poll, and JSON IPC.

The crate wraps `AXUIElement` with convenience methods (`title()`, `pid()`,
`role()`, `children()`) and a generic `AXAttribute::new()` for attributes
without dedicated accessors. Our key queries:

1. `AXUIElement::system_wide()` → focused application via
   `AXAttribute::new("AXFocusedApplication")`
2. Application element → app name via `title()`
3. Application element → focused window via
   `AXAttribute::new("AXFocusedWindow")` → window title via `title()`
4. System-wide → focused element via `AXAttribute::new("AXFocusedUIElement")`
   → selected text via `AXAttribute::new("AXSelectedText")`

If `title()` on the application element proves unreliable for some apps,
fall back to `pid()` + `NSRunningApplication` via the `cocoa`/`objc` crates
already in the transitive dependency tree.

`is_available()` calls `AXIsProcessTrusted()` from `accessibility-sys`.

**Latency budget**: <1ms per query (in-process function call vs 15-40ms for
fork+exec). At 200ms polling interval, negligible overhead.

**macOS-only dependency**:
```toml
[target.'cfg(target_os = "macos")'.dependencies]
accessibility = "0.2"
```

### A3. External capture thread (`ext_capture.rs`)

The polling/dwell/dedup logic lives in `ext_capture.rs`, OS-agnostic:

- Calls `platform_source()` at startup; if `None`, returns immediately
- Checks `source.is_available()`; if false, prints a one-time permission
  warning and returns (graceful degradation)
- Spawns a thread that polls `source.query()` every 200ms
- Emits `Event::ExternalSelection` when selected text changes
- Dwell logic: only emit after selection stabilizes for ~300ms (same concept as
  cursor dwell in editor_capture)
- Dedup: suppress if selected text is identical to the previous emission

**Filtering**:
- Skip when frontmost app matches `ax_ignore_apps` (case-insensitive)
- Default: `["Zed"]` — Zed uses GPUI, so AX can't read from its panes anyway,
  and regular Zed files are already covered by EditorSnapshot
- Config: `ext_ignore_apps = ["Zed"]` in config.toml (optional, sensible default)

**Integration into CaptureHandle**:
- Add a third thread alongside `editor_thread` and `diff_thread`
- New `ext_events: Arc<Mutex<Vec<Event>>>` in CaptureHandle
- `drain()` and `collect()` return all three event streams

### A4. Permissions UX

The Accessibility permission is granted to the *terminal emulator* (iTerm2),
not to attend itself, because macOS TCC traces the "responsible process" up
the process tree. Most developers using accessibility-aware tools have already
granted this.

Permission checking is part of the `ExternalSource` trait (`is_available()`).
The macOS backend implements this via `AXIsProcessTrusted()`. The ext_capture
thread checks `is_available()` once at startup:
- If false, prints a one-time message to stderr:
  "External text capture requires Accessibility permission for your terminal.
  Grant it in System Settings > Privacy & Security > Accessibility."
- Gracefully degrades: thread returns, no events emitted
- Future platforms implement their own permission checks in `is_available()`

### A4. Per-app reliability (empirically verified)

Tested via JXA/AppleScript probing of the macOS accessibility tree:

| App | AXSelectedText | AXValue | URL | Window title |
|-----|---------------|---------|-----|-------------|
| iTerm2 | **Works** | Full terminal buffer | N/A | Tab name |
| Firefox | **Broken** | Not exposed | Not exposed | Page title (in window name) |
| Safari | **Works** | N/A | Via AppleScript | Page title |
| Chrome | Works (per docs) | N/A | N/A | Page title (in window name) |
| VS Code | Works (per docs) | N/A | N/A | File path (in window name) |
| Zed | **Broken** | Not exposed | N/A | Workspace name |
| Slack | Partial | N/A | N/A | Channel name |

**Zed findings** (confirmed during attend development):
- Zed uses GPUI (custom GPU rendering), not standard AppKit `NSTextView`
- GPUI does not expose `AXSelectedText` to the macOS accessibility tree
- The AX API cannot read content from Zed's editor panes
- Window title is accessible (workspace name), but not useful for content
- See `plans/phase-12-zed-diff.md` for the Zed diff view gap analysis

**Firefox findings** (empirically confirmed 2026-02-20):
- The AXWebArea element exists in the tree (role=AXWebArea, desc="page title")
- `AXSelectedText` is not exposed as an attribute on any element
- `AXSelectedTextMarkerRange` is null even when text is visually selected
- `AXURL` is declared but errors when read (AppleEvent handler failed)
- Firefox's own AppleScript dictionary is minimal: `document: null`, no tabs
- The page title IS available, embedded in the window name:
  `"Wikipedia, the free encyclopedia — Original profile"` (parseable by
  stripping the ` — <profile>` suffix)

**iTerm2 findings** (empirically confirmed 2026-02-20):
- Focused element is `AXTextArea` with role description "shell"
- `AXSelectedText` works: returns selected terminal text (empty when nothing selected)
- `AXValue` returns the **entire visible terminal buffer** (hundreds of lines)
- This means attend could optionally capture terminal context beyond just
  selected text, though the full buffer is too large to emit routinely

**Safari** is the recommended testbed for AX development (confirmed working).
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

### No install step for external capture

The platform backend compiles as part of `cargo build` (cfg-gated deps). No
separate binary, no user-facing install step. On macOS, the Accessibility
permission prompt is triggered on first use by the terminal emulator. Future
platforms may have their own permission models handled via `is_available()`.

---

## Task breakdown

### Phase 12a: External selection capture (Part A)

| # | Task | Depends on |
|---|------|-----------|
| A1 | `ExternalSource` trait, `ExternalSnapshot`, `platform_source()` dispatch | — |
| A2 | macOS backend (`ext_capture/macos.rs`): AX queries via `accessibility` crate | A1 |
| A3 | New `Event::ExternalSelection` variant + serde | — |
| A4 | Polling thread, `DwellTracker`, dedup logic in `ext_capture.rs` | A1, A3 |
| A5 | Wire ext_capture into `CaptureHandle` (third thread) | A4 |
| A6 | `render.rs`: render ExternalSelection as blockquote | A3 |
| A7 | `merge.rs`: compress consecutive same-app selections | A3 |
| A8 | `receive.rs`: pass ExternalSelection through filter unchanged | A3 |
| A9 | Config: `ext_ignore_apps` | A4 |
| A10 | Tests: DwellTracker unit tests, merge/compress prop tests, render tests | A7 |

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
