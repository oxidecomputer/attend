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
use crate::narrate::{cache_dir, pending_dir};
use crate::state;
use crate::util;

/// The JSON message sent by the browser extension.
#[derive(Debug, Deserialize)]
struct BridgeMessage {
    /// The selected text.
    text: String,
    /// Page URL.
    url: String,
    /// Page title.
    title: String,
    /// Whether the selection is inside a `<code>`/`<pre>` block.
    #[serde(default)]
    is_code: bool,
}

/// Run the browser bridge: read one native messaging message, write a
/// `BrowserSelection` event to the active session's pending directory.
pub(super) fn run() -> anyhow::Result<()> {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    // Read one message from the browser extension.
    let msg: BridgeMessage =
        native_messaging::host::recv_json(&mut stdin, native_messaging::host::MAX_FROM_BROWSER)?;

    // Skip empty selections.
    if msg.text.trim().is_empty() {
        native_messaging::host::send_json(&mut stdout, &serde_json::json!({"status": "skipped"}))?;
        return Ok(());
    }

    // Find the active session.
    let Some(session_id) = state::listening_session() else {
        // No active session: acknowledge and exit silently.
        native_messaging::host::send_json(
            &mut stdout,
            &serde_json::json!({"status": "no_session"}),
        )?;
        return Ok(());
    };

    // Write the event to the pending directory.
    let events = vec![Event::BrowserSelection {
        // Browser bridge events don't participate in the recording timeline,
        // so offset_secs is set to 0. The receive pipeline sorts by file
        // timestamp, not by offset_secs, for cross-process events.
        offset_secs: 0.0,
        url: msg.url,
        title: msg.title,
        text: msg.text,
        is_code: msg.is_code,
    }];

    let json = serde_json::to_string(&events)?;
    let ts = util::utc_now().replace(':', "-");
    let dir = pending_dir(&session_id);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{ts}.json"));
    util::atomic_write_str(&path, &json)?;

    native_messaging::host::send_json(&mut stdout, &serde_json::json!({"status": "ok"}))?;

    // Touch the cache dir to signal the listener that new events are available.
    // (The listener polls the pending dir, so this is a no-op hint.)
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
            "text": "pub fn spawn(&mut self)",
            "url": "https://docs.rs/tokio/latest/tokio/process/",
            "title": "tokio::process - Rust",
            "is_code": true
        }"#;
        let msg: BridgeMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text, "pub fn spawn(&mut self)");
        assert_eq!(msg.url, "https://docs.rs/tokio/latest/tokio/process/");
        assert_eq!(msg.title, "tokio::process - Rust");
        assert!(msg.is_code);
    }

    /// is_code defaults to false when not present.
    #[test]
    fn deserialize_bridge_message_no_is_code() {
        let json = r#"{"text": "hello", "url": "https://example.com", "title": "Example"}"#;
        let msg: BridgeMessage = serde_json::from_str(json).unwrap();
        assert!(!msg.is_code);
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
