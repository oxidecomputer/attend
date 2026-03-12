use super::*;

/// BridgeMessage deserializes correctly from extension JSON (without plain_text).
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
    assert_eq!(msg.plain_text, "", "missing plain_text defaults to empty");
}

/// BridgeMessage deserializes plain_text when present.
#[test]
fn deserialize_bridge_message_with_plain_text() {
    let json = r#"{
            "html": "<strong>bold</strong> text",
            "plain_text": "bold text",
            "url": "https://example.com",
            "title": "Example"
        }"#;
    let msg: BridgeMessage = serde_json::from_str(json).unwrap();
    assert_eq!(msg.plain_text, "bold text");
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
