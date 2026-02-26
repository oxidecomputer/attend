//! Native messaging host for browser extensions.
//!
//! The browser extension sends selection events as length-prefixed JSON on
//! stdin, and this process writes them to the active session's pending
//! directory (the same directory used by the recording daemon). The receive
//! pipeline picks them up and delivers them to the agent.
//!
//! This is a one-shot process: read one message, write the event, respond,
//! exit. Firefox launches a new process for each `sendNativeMessage` call.

use std::fs;
use std::io;

use serde::Deserialize;

use crate::narrate::merge::Event;
use crate::narrate::{browser_staging_dir, cache_dir};
use crate::state;
use crate::util;

/// The JSON message sent by the browser extension.
#[derive(Debug, Deserialize)]
struct BridgeMessage {
    /// The selected content as an HTML fragment.
    html: String,
    /// Page URL.
    url: String,
    /// Page title.
    title: String,
}

/// Convert an HTML fragment to markdown using htmd.
fn html_to_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_else(|_| html.to_string())
}

/// Run the browser bridge: read one native messaging message, write a
/// `BrowserSelection` event to the active session's pending directory.
pub(super) fn run() -> anyhow::Result<()> {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    // Read one message from the browser extension.
    let msg: BridgeMessage =
        native_messaging::host::recv_json(&mut stdin, native_messaging::host::MAX_FROM_BROWSER)?;

    // Convert HTML to markdown.
    let text = html_to_markdown(&msg.html);

    // Skip empty selections.
    if text.trim().is_empty() {
        native_messaging::host::send_json(&mut stdout, &serde_json::json!({"status": "skipped"}))?;
        return Ok(());
    }

    // Only stage events while narration is actively recording.
    // The record lock exists only while the recording daemon is running.
    if !crate::narrate::record_lock_path().exists() {
        native_messaging::host::send_json(
            &mut stdout,
            &serde_json::json!({"status": "not_recording"}),
        )?;
        return Ok(());
    }

    // Resolve the session, if any. When no agent session is active the
    // event is staged to the `_local` directory so it is still captured.
    let session_id = state::listening_session();

    // Stage the event for collection by the recording daemon.
    // Browser selections are not delivered directly to the agent; they
    // accumulate in a staging directory and are included when the user
    // manually concludes narration (stop/flush).
    let now = chrono::Utc::now();
    let events = vec![Event::BrowserSelection {
        // Placeholder: the recording daemon overwrites this with the
        // UTC timestamp parsed from the staging filename.
        timestamp: now,
        last_seen: now,
        url: msg.url,
        title: msg.title,
        text,
    }];

    let json = serde_json::to_string(&events)?;
    let ts = util::utc_now_nanos().replace(':', "-");
    let id = uuid::Uuid::new_v4();
    let dir = browser_staging_dir(session_id.as_ref());
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{ts}-{id}.json"));
    util::atomic_write_str(&path, &json)?;

    native_messaging::host::send_json(&mut stdout, &serde_json::json!({"status": "ok"}))?;

    let _ = fs::File::open(cache_dir());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BridgeMessage deserializes correctly from extension JSON.
    #[test]
    fn deserialize_bridge_message() {
        let json = r#"{
            "html": "<code>pub fn spawn(&amp;mut self)</code>",
            "url": "https://docs.rs/tokio/latest/tokio/process/",
            "title": "tokio::process - Rust"
        }"#;
        let msg: BridgeMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.html, "<code>pub fn spawn(&amp;mut self)</code>");
        assert_eq!(msg.url, "https://docs.rs/tokio/latest/tokio/process/");
        assert_eq!(msg.title, "tokio::process - Rust");
    }

    /// HTML is converted to markdown by the bridge.
    #[test]
    fn html_to_markdown_basic() {
        assert_eq!(html_to_markdown("<strong>bold</strong>"), "**bold**");
        assert_eq!(
            html_to_markdown(r#"<a href="https://example.com">link</a>"#),
            "[link](https://example.com)"
        );
        assert_eq!(html_to_markdown("<code>foo()</code>"), "`foo()`");
    }

    /// Code blocks with language hints are converted to fenced blocks.
    #[test]
    fn html_to_markdown_code_block() {
        let html = r#"<pre><code class="language-rust">fn main() {}</code></pre>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("```rust"), "should have language hint: {md:?}");
        assert!(md.contains("fn main() {}"), "should have code: {md:?}");
    }

    /// Plain text without HTML passes through.
    #[test]
    fn html_to_markdown_plain_text() {
        assert_eq!(html_to_markdown("just plain text"), "just plain text");
    }

    /// Native messaging round-trip: encode a message, decode it back.
    #[test]
    fn native_messaging_round_trip() {
        let original = serde_json::json!({"status": "ok"});
        let encoded = native_messaging::host::encode_message(&original).unwrap();
        let decoded: serde_json::Value = native_messaging::host::recv_json(
            &mut &encoded[..],
            native_messaging::host::MAX_FROM_BROWSER,
        )
        .unwrap();
        assert_eq!(original, decoded);
    }
}
