use super::*;

#[test]
fn is_attend_prompt_exact() {
    let json = serde_json::json!({"prompt": "/attend", "session_id": "abc"});
    assert!(is_attend_prompt(&json));
}

#[test]
fn is_attend_prompt_with_whitespace() {
    let json = serde_json::json!({"prompt": "  /attend  ", "session_id": "abc"});
    assert!(is_attend_prompt(&json));
}

#[test]
fn is_attend_prompt_different_text() {
    let json = serde_json::json!({"prompt": "hello world", "session_id": "abc"});
    assert!(!is_attend_prompt(&json));
}

#[test]
fn is_attend_prompt_no_prompt_field() {
    let json = serde_json::json!({"session_id": "abc"});
    assert!(!is_attend_prompt(&json));
}

#[test]
fn is_attend_prompt_partial() {
    let json = serde_json::json!({"prompt": "/attend to this", "session_id": "abc"});
    assert!(!is_attend_prompt(&json));
}
