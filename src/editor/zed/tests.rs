use super::*;

#[test]
fn strip_json_comments_full_line() {
    let input = "// This is a comment\n{\"key\": \"value\"}\n";
    let result = strip_json_comments(input);
    assert_eq!(result, "{\"key\": \"value\"}\n");
}

#[test]
fn strip_json_comments_inline() {
    let input = "{\"key\": \"value\"} // trailing comment\n";
    let result = strip_json_comments(input);
    assert_eq!(result, "{\"key\": \"value\"} \n");
}

#[test]
fn strip_json_comments_inside_string_preserved() {
    let input = "{\"url\": \"https://example.com\"}\n";
    let result = strip_json_comments(input);
    assert_eq!(result, "{\"url\": \"https://example.com\"}\n");
}

#[test]
fn strip_json_comments_empty() {
    let result = strip_json_comments("");
    assert_eq!(result, "");
}

#[test]
fn strip_json_comments_mixed() {
    let input = "// header comment\n{\n  // inner comment\n  \"a\": 1\n}\n";
    let result = strip_json_comments(input);
    assert_eq!(result, "{\n  \"a\": 1\n}\n");
}

#[test]
fn find_line_comment_none_when_absent() {
    assert_eq!(find_line_comment("{\"key\": \"value\"}"), None);
}

#[test]
fn find_line_comment_finds_comment() {
    assert_eq!(find_line_comment("code // comment"), Some(5));
}

#[test]
fn find_line_comment_ignores_url_in_string() {
    assert_eq!(find_line_comment("\"url\": \"https://example.com\""), None);
}

#[test]
fn find_line_comment_after_string_with_slash() {
    assert_eq!(find_line_comment("\"path\": \"a/b\" // comment"), Some(14));
}

#[test]
fn parse_jsonc_trailing_commas() {
    let input = "[{\"a\": 1}, {\"b\": 2},]";
    let parsed: Vec<serde_json::Value> = parse_jsonc(input).unwrap();
    assert_eq!(parsed.len(), 2);
}

#[test]
fn strip_trailing_commas_in_object() {
    let input = "{\"a\": 1, \"b\": 2,}";
    let result = strip_trailing_commas(input);
    let _: serde_json::Value = serde_json::from_str(&result).unwrap();
}

#[test]
fn parse_jsonc_with_comments_and_trailing_commas() {
    let input = "// Zed config\n[\n  {\"a\": 1},\n  // second entry\n  {\"b\": 2},\n]\n";
    let parsed: Vec<serde_json::Value> = parse_jsonc(input).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0]["a"], 1);
    assert_eq!(parsed[1]["b"], 2);
}

#[test]
fn is_dictation_keybinding_current_keys() {
    let flush = serde_json::json!({"bindings": {"cmd-:": ["task::Spawn", {"task_name": "Flush"}]}});
    let toggle =
        serde_json::json!({"bindings": {"cmd-;": ["task::Spawn", {"task_name": "Toggle"}]}});
    assert!(is_dictation_keybinding(&flush));
    assert!(is_dictation_keybinding(&toggle));
}

#[test]
fn is_dictation_keybinding_rejects_multi_binding() {
    let multi = serde_json::json!({"bindings": {"cmd-:": ["task::Spawn"], "cmd-k": ["other"]}});
    assert!(!is_dictation_keybinding(&multi));
}

#[test]
fn is_dictation_keybinding_rejects_unrelated() {
    let other = serde_json::json!({"bindings": {"cmd-s": ["editor::Save"]}});
    assert!(!is_dictation_keybinding(&other));
}
