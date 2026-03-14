# Phase 19: Clipboard Capture

**Dependencies**: Phase 12a (external selection infrastructure).
**Effort**: Small-Medium | **Risk**: Low

---

## Motivation

External selection capture relies on the platform accessibility API to read
selected text from the frontmost application. This works well in general, but
has gaps:

- **Ignored apps** (e.g., Zed is in `ext_ignore_apps` because we already
  capture rich context via the editor integration): accessibility-based
  selection capture is redundant and disabled, so ad-hoc copies from the
  editor that fall outside snapshot/diff coverage are invisible.
- **Non-text-selection copies**: content copied from dialogs, address bars,
  context menus, or other UI elements that don't surface as accessibility
  selections.
- **Platform limitations**: some apps or toolkits simply don't implement the
  accessibility text selection interface.
- **Non-text content**: screenshots, diagrams, and other images copied to
  the clipboard are invisible to all existing capture mechanisms.

Clipboard capture fills these holes. When the user copies content (Cmd+C /
Ctrl+C), we capture it — text inline, images as staged files. If a richer
source (ExternalSelection with app context, BrowserSelection with URL
context) already captured the same text, the clipboard version is dropped
during merge. In all cases, the richer source wins.

`arboard` (already in the dependency tree from Phase 16) supports both
`get_text()` and `get_image()` cross-platform. The `image` crate is already
a transitive dependency of `arboard` — we promote it to a direct dependency
for PNG encoding.

---

## Design

### Event representation

New `Event` variant:

```rust
Event::ClipboardSelection {
    timestamp: DateTime<Utc>,
    content: ClipboardContent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipboardContent {
    Text { text: String },
    Image { path: String },
}
```

No `last_seen` field — clipboard captures are point-in-time only. The
clipboard tracker emits exactly once per clipboard change. If the clipboard
content doesn't change between polls, no event is emitted.

No `app` or `window_title` — the clipboard is unlabeled by nature. We don't
know which app originated the copy, and guessing from the frontmost app
would be unreliable (the copy could have happened via a global hotkey,
script, or background process).

**Text content** is stored inline in the event, with no size limit.

**Image content** is encoded to PNG and staged to a file. The event stores
the path to the staged file. The staged file lives in
`<cache_dir>/attend/clipboard-staging/` alongside other staging directories,
using the same nanosecond-timestamp + UUID filename convention:

```
~/.cache/attend/clipboard-staging/<timestamp>-<uuid>.png
```

The `clipboard-staging/` directory is not session-scoped (unlike
`browser-staging/<session-key>/`) because the clipboard polling thread runs
inside the daemon and writes directly — no external process needs to
discover the session key.

### Change detection

**Text**: compare against previous text content. Emit on change.

**Images**: computing a pixel-level diff every 500ms is wasteful. Instead,
compare the **byte length** of the raw RGBA buffer. If the length changes
(different image dimensions), that's a definite change. If the length is the
same, compute a fast hash (e.g., `xxhash` or `fxhash` — already common in
Rust ecosystems, or just use `std::hash`'s default hasher on a sampled
subset of pixels) to detect content changes without comparing every byte.
On change → encode PNG → stage → emit event.

The tracker state machine tracks *which kind* of content was last seen
(text vs. image vs. empty) to detect cross-type changes (e.g., user copies
text then copies a screenshot).

### Polling thread

A new fourth capture thread in `CaptureHandle`, polling at **500ms**
intervals (clipboard changes less frequently than accessibility selections;
no need for 200ms). The thread:

1. Creates its own `arboard::Clipboard` instance (not `Send`).
2. Seeds with the current clipboard content on start (**no event emitted**
   for pre-existing content — we only capture changes during the session).
3. Each tick: try `get_text()`, then `get_image()`. First success wins
   (text takes priority if both are available, which can happen on some
   platforms when copying formatted text). On change → emit event. On
   error → skip silently.
4. Respects `stop_flag` and `paused_flag` (same as ext_capture).

**Guardrails**:
- **Whitespace-only text**: skip content that is empty or whitespace-only
  after trimming.
- **Initialization failure**: if `arboard::Clipboard::new()` fails (e.g.,
  headless environment), log a warning and exit the thread gracefully.
  `CaptureHandle` stores `None` for the thread handle, same as ext_capture
  on non-macOS platforms.
- **PNG encoding failure**: log and skip (should not happen with valid RGBA
  data, but don't crash).

### Merge dedup: clipboard loses to richer sources

A new dedup pass in `compress_and_merge`, after `dedup_browser_vs_external`:

```
dedup_clipboard_selections(events)
```

**Logic**: drop any text `ClipboardSelection` whose normalized text matches
any:
- `ExternalSelection` (both are plain text; normalize and compare)
- `BrowserSelection` (compare against `plain_text` field; see below)

**Normalization**: collapse all whitespace (newlines, tabs, runs of spaces)
to single spaces, then trim. Applied to both sides of every comparison.

Content-based matching within the flush period. No time window needed —
all events in a single `compress_and_merge` call are from the same recording
period (a few minutes at most), so temporal proximity is structural.

Image `ClipboardSelection` events are never deduped against other types
(no other source captures images).

#### Browser plain-text for dedup

When the user copies rich text from the browser, the clipboard gets a
plain-text representation while `BrowserSelection` gets HTML→markdown.
These won't match on exact comparison (`[link](url)` vs `link`). To
handle this, we fork at the HTML source:

- The browser extension already has the DOM selection. Alongside the
  existing HTML→markdown conversion, it also emits
  `plain_text: selection.toString()` (the browser's own plain-text
  rendering of the selection).
- `Event::BrowserSelection` gains a new `plain_text: String` field.
- The dedup pass compares `normalize(clipboard.text)` against
  `normalize(browser.plain_text)`.

This adds one field to the staged JSON event and one `toString()` call
in the extension — negligible cost, reliable dedup.

The existing `dedup_browser_vs_external` pass should also be updated to
compare `normalize(browser.plain_text)` against
`normalize(external.text)`, which is more robust than the current
trimmed-text comparison for rich-text selections.

Progressive selection subsumption already handles the substring case (e.g.,
user copies a partial selection that was captured in full by ext_capture),
but text `ClipboardSelection` needs to participate in
`subsume_progressive_selections` alongside the existing variants. The
subsumption rule is one-directional: a `ClipboardSelection` can be subsumed
by any richer selection type, and a `ClipboardSelection` can subsume
another `ClipboardSelection`, but a `ClipboardSelection` should not subsume
a richer type even if it contains the text.

### Render

**Text** `ClipboardSelection` renders as a plain blockquote with no
attribution:

```markdown
> some copied text that wasn't captured
> by any other selection source
```

**Image** `ClipboardSelection` renders as a markdown image tag:

```markdown
![clipboard](/Users/oxide/.cache/attend/clipboard-staging/1740000000000-abc123.png)
```

Claude Code's `Read` tool can read image files natively, so the agent can
view the image when it encounters this tag. The path is absolute so it
works regardless of the agent's working directory.

The narration protocol tells the agent that any blockquote without
attribution is a clipboard text selection, and that `![clipboard](...)`
tags are clipboard images.

Like `ExternalSelection` and `BrowserSelection`, clipboard selections are
not snipped (they capture ephemeral state that cannot be reconstructed).

### Receive/filter

`ClipboardSelection` always passes through filtering (no path scoping),
same as `ExternalSelection` and `BrowserSelection`. No relativization
needed (no project-relative path fields — the image staging path is a
cache directory, not a project path).

### Config

New optional field in `Config`:

```toml
clipboard_capture = false   # default: true
```

`clipboard_capture: Option<bool>` (default `true` via `serde(default)`).
When `false`, the clipboard polling thread is not spawned.

### Agent permissions

The install step (`agent/claude/settings/install.rs`) already pre-authorizes
`Bash(attend look:*)` and `Bash(attend listen:*)`. Add a `Read` permission
for the clipboard staging directory so the agent can read staged images
without prompting:

```
Read(<cache_dir>/attend/clipboard-staging/*)
```

The path is computed at install time from `state::cache_dir()`, same as
`bin_cmd` is resolved dynamically. The uninstall step should remove this
permission alongside the existing ones.

A `clipboard_staging_dir()` function is added to `narrate.rs` alongside
`browser_staging_dir()` and `shell_staging_dir()`.

### Documentation

**Narration protocol** (`src/agent/messages/narration_protocol.md`): add
`ClipboardSelection` as the eighth event type. Describe both text form
(plain blockquote, no attribution) and image form (`![clipboard](path)`).

**Setup guide** (`docs/setup.md`): mention clipboard capture in the
configuration section, noting the `clipboard_capture` config key.

**README** (`README.md`): no change needed — clipboard capture is automatic
and has no user-facing setup steps.

### Cleanup

Staged clipboard images are ephemeral. They are cleaned up:
- By the daemon on stop/flush (same lifecycle as browser-staging).
- By `archive_retention` cleanup for old sessions.

---

## Task breakdown

Red-green TDD: write tests early, stub just enough to compile (red), then
implement to make them pass (green). Tasks are ordered so tests come first
within each area, and stubs are the minimum needed for compilation.

### Phase A: Skeleton + types (compiles, tests red)

| # | Task | Depends On | Files |
|---|------|------------|-------|
| 1 | Promote `image` to direct dependency | — | `Cargo.toml` |
| 2 | `Event::ClipboardSelection` + `ClipboardContent` enum + serde | — | `narrate/merge.rs` |
| 3 | `plain_text` field on `BrowserSelection` (with `serde(default)`) | — | `narrate/merge.rs` |
| 4 | Stub `ClipboardTracker` + `clipboard_capture` module (public types, empty impls) | 2 | new `narrate/clipboard_capture.rs` |
| 5 | Stub `clipboard_staging_dir()` | — | `narrate.rs` |
| 6 | Stub `dedup_clipboard_selections` + `normalize_text` (no-op bodies) | 2 | `narrate/merge.rs` |
| 7 | Stub render arm for `ClipboardSelection` (empty match arm) | 2 | `narrate/render.rs` |
| 8 | Stub filter pass-through for `ClipboardSelection` | 2 | `narrate/receive/filter.rs` |
| 9 | `clipboard_capture` config option | — | `config.rs` |

### Phase B: Tests (compiles, all new tests red)

| # | Task | Depends On | Files |
|---|------|------------|-------|
| 10 | Write serde tests: `clipboard_text_roundtrip`, `clipboard_image_roundtrip`, `browser_plain_text_roundtrip`, `browser_plain_text_default` | 2, 3 | `narrate/merge/tests.rs` |
| 11 | Write tracker tests: `seed_does_not_emit`, `text_change_emits`, `same_text_does_not_repeat`, `whitespace_only_skipped`, `empty_to_text_emits`, `text_to_image_emits`, `image_to_text_emits`, `image_dimension_change_emits`, `same_image_does_not_repeat`, `both_unavailable_skips` | 4 | `narrate/clipboard_capture.rs` |
| 12 | Write dedup tests: `clipboard_deduped_by_exact_external`, `clipboard_deduped_by_exact_browser`, `clipboard_deduped_by_normalized_whitespace`, `clipboard_no_match_retained`, `clipboard_image_never_deduped`, `clipboard_substring_not_deduped` | 6 | `narrate/merge/tests.rs` |
| 13 | Write browser dedup test: `browser_vs_external_uses_normalized_plain_text` | 6 | `narrate/merge/tests.rs` |
| 14 | Write subsumption tests: `clipboard_subsumed_by_external`, `clipboard_subsumed_by_browser`, `clipboard_subsumes_clipboard`, `clipboard_does_not_subsume_external`, `clipboard_does_not_subsume_browser` | 6 | `narrate/merge/tests.rs` |
| 15 | Write render tests: `clipboard_text_renders_as_plain_blockquote`, `clipboard_text_multiline`, `clipboard_image_renders_as_image_tag`, `clipboard_no_attribution_between_attributed` | 7 | `narrate/render/tests.rs` |
| 16 | Write filter tests: `clipboard_passes_through_filter`, `clipboard_not_relativized` | 8 | `narrate/receive/filter/tests.rs` |
| 17 | Write config tests: `clipboard_capture_defaults_to_true`, `clipboard_capture_explicit_false` | 9 | `config.rs` |
| 18 | Write prop tests: `prop_clipboard_in_merge_pipeline`, `prop_clipboard_subsumption_asymmetric` | 6 | `narrate/merge/tests.rs` |

### Phase C: Implementation (tests go green)

| # | Task | Depends On | Files |
|---|------|------------|-------|
| 19 | Implement `ClipboardTracker` (text + image change detection) | 11 | `narrate/clipboard_capture.rs` |
| 20 | Implement `clipboard_staging_dir()` | — | `narrate.rs` |
| 21 | Implement `normalize_text` + `dedup_clipboard_selections` | 12, 13 | `narrate/merge.rs` |
| 22 | Update `dedup_browser_vs_external` to use normalized `plain_text` | 13 | `narrate/merge.rs` |
| 23 | Implement clipboard subsumption participation (one-directional) | 14 | `narrate/merge.rs` |
| 24 | Implement render: text as plain blockquote, image as `![clipboard](path)` | 15 | `narrate/render.rs` |
| 25 | Implement filter pass-through for `ClipboardSelection` | 16 | `narrate/receive/filter.rs` |
| 26 | Browser extension: emit `plain_text: selection.toString()` | — | browser extension `content.js` |

### Phase D: Wiring + docs

| # | Task | Depends On | Files |
|---|------|------------|-------|
| 27 | Implement `clipboard_capture::spawn()` polling thread | 19 | `narrate/clipboard_capture.rs` |
| 28 | Wire into `CaptureHandle` as fourth thread | 27 | `narrate/capture.rs` |
| 29 | Wire clipboard events into daemon collection pipeline + staging cleanup | 28 | `narrate/record.rs` |
| 30 | Pre-authorize `Read` on clipboard staging dir in install + uninstall | 20 | `agent/claude/settings/install.rs`, `agent/claude/settings/uninstall.rs` |
| 31 | Update narration protocol + setup guide | 24 | `agent/messages/narration_protocol.md`, `docs/setup.md` |

---

## Test plan (red-green)

Tests are written first (red), then implementation makes them pass (green).
Each test name states the invariant it asserts.

### ClipboardTracker (`narrate/clipboard_capture.rs`)

| Test | Invariant |
|------|-----------|
| `seed_does_not_emit` | Initializing the tracker with current clipboard content produces no event. |
| `text_change_emits` | When clipboard text changes from A to B, a `ClipboardContent::Text` event is emitted. |
| `same_text_does_not_repeat` | Polling the same text content twice produces only one event. |
| `whitespace_only_skipped` | Clipboard containing only whitespace/newlines produces no event. |
| `empty_to_text_emits` | Transitioning from empty/error clipboard to text emits. |
| `text_to_image_emits` | Switching from text to image content emits an `Image` event. |
| `image_to_text_emits` | Switching from image to text content emits a `Text` event. |
| `image_dimension_change_emits` | Image with different dimensions than previous emits a new event. |
| `same_image_does_not_repeat` | Polling identical image data twice produces only one event. |
| `both_unavailable_skips` | When both `get_text` and `get_image` return `ContentNotAvailable`, no event is emitted. |

### Merge dedup (`narrate/merge/tests.rs`)

**`dedup_clipboard_selections`:**

| Test | Invariant |
|------|-----------|
| `clipboard_deduped_by_exact_external` | Clipboard text matching an ExternalSelection's text is dropped. |
| `clipboard_deduped_by_exact_browser` | Clipboard text matching a BrowserSelection's `plain_text` is dropped. |
| `clipboard_deduped_by_normalized_whitespace` | Clipboard `"foo  bar\n baz"` is deduped against external `"foo bar baz"` after normalization. |
| `clipboard_no_match_retained` | Clipboard text not matching any other selection survives. |
| `clipboard_image_never_deduped` | `ClipboardContent::Image` is never dropped by selection dedup. |
| `clipboard_substring_not_deduped` | Clipboard text that is a substring of (but not equal to) another selection is NOT dropped by dedup (that's subsumption's job). |

**`dedup_browser_vs_external` (updated):**

| Test | Invariant |
|------|-----------|
| `browser_vs_external_uses_normalized_plain_text` | ExternalSelection is dropped when its normalized text matches the BrowserSelection's normalized `plain_text`, even if the markdown `text` differs. |

**Subsumption:**

| Test | Invariant |
|------|-----------|
| `clipboard_subsumed_by_external` | ExternalSelection containing clipboard text within the subsumption window causes clipboard to be dropped. |
| `clipboard_subsumed_by_browser` | BrowserSelection whose `plain_text` contains clipboard text causes clipboard to be dropped. |
| `clipboard_subsumes_clipboard` | Later ClipboardSelection containing earlier clipboard's text within the subsumption window causes earlier to be dropped. |
| `clipboard_does_not_subsume_external` | ClipboardSelection does not subsume an ExternalSelection, even if the clipboard text contains the external's text. |
| `clipboard_does_not_subsume_browser` | ClipboardSelection does not subsume a BrowserSelection, even if the clipboard text contains the browser's `plain_text`. |

### Render (`narrate/render/tests.rs`)

| Test | Invariant |
|------|-----------|
| `clipboard_text_renders_as_plain_blockquote` | `ClipboardContent::Text` renders as `> ` lines with no attribution header. |
| `clipboard_text_multiline` | Multi-line clipboard text renders each line as a separate `> ` line. |
| `clipboard_image_renders_as_image_tag` | `ClipboardContent::Image` renders as `![clipboard](/path/to/file.png)`. |
| `clipboard_no_attribution_between_attributed` | A clipboard blockquote between an ExternalSelection and BrowserSelection has no attribution, while its neighbors do. |

### Receive/filter (`narrate/receive/filter/tests.rs`)

| Test | Invariant |
|------|-----------|
| `clipboard_passes_through_filter` | ClipboardSelection is not dropped or redacted regardless of cwd scope. |
| `clipboard_not_relativized` | ClipboardSelection (including image path) is untouched by relativization. |

### Event serde (`narrate/merge/tests.rs`)

| Test | Invariant |
|------|-----------|
| `clipboard_text_roundtrip` | `ClipboardSelection` with `Text` content survives JSON serialize/deserialize. |
| `clipboard_image_roundtrip` | `ClipboardSelection` with `Image` content survives JSON serialize/deserialize. |
| `browser_plain_text_roundtrip` | `BrowserSelection` with `plain_text` field survives JSON round-trip. |
| `browser_plain_text_default` | Deserializing a `BrowserSelection` from an old archive (no `plain_text` key) produces an empty string default. |

### Prop tests (`narrate/merge/tests.rs`)

| Test | Invariant |
|------|-----------|
| `prop_clipboard_in_merge_pipeline` | Extend the merge pipeline prop test's event generator to include `ClipboardSelection` (text and image). After `compress_and_merge`: no text clipboard survives whose normalized text equals any ExternalSelection or BrowserSelection `plain_text` in the output. |
| `prop_clipboard_subsumption_asymmetric` | Generate random clipboard + richer selection pairs. After subsumption: a clipboard event may be dropped when a richer type contains it, but a richer type is never dropped by a clipboard event containing it. |

### Config (`config.rs`)

| Test | Invariant |
|------|-----------|
| `clipboard_capture_defaults_to_true` | A default `Config` has `clipboard_capture` effectively true. |
| `clipboard_capture_explicit_false` | Parsing `clipboard_capture = false` from TOML yields false. |

---

## Verification

- Start recording in Zed (ignored app), select text, Cmd+C → clipboard
  event captured. Rendered as plain blockquote in narration output.
- Start recording, select text in Terminal (not ignored) → ExternalSelection
  captured with app context. Then Cmd+C → clipboard change detected, but
  deduped against the existing ExternalSelection during merge.
- Start recording, copy URL from browser address bar (not a text selection)
  → clipboard event captured (browser extension only captures page
  selections, not address bar copies).
- Start recording, copy an image to clipboard (e.g., select an image in
  a web page and Cmd+C, or copy from Preview) → PNG staged,
  `![clipboard](path)` in narration output.
- Start recording, copy nothing → no clipboard events emitted.
- Start recording, copy large file contents → captured in full (no size
  limit).
- Set `clipboard_capture = false` in config → no clipboard thread spawned.

---

## Non-goals

- **Clipboard history**: we capture point-in-time changes, not a clipboard
  manager. No persistence beyond the recording session.
- **Rich text / HTML**: we capture plain text and images. Formatted text
  (HTML, RTF) is captured as plain text via `get_text()`.
- **File references**: clipboard "file copy" operations (e.g., Cmd+C on a
  file in Finder) are platform-specific pasteboard types that `arboard`
  doesn't expose. Out of scope.
- **Write-back**: clipboard capture is read-only. We never modify clipboard
  contents (that's Phase 16's yank feature, completely separate).
