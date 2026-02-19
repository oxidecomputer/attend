use super::*;

// -- Zed DB fixture tests --

/// Create an in-memory SQLite DB with Zed's schema for testing `query_editors`.
fn create_test_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items (
            workspace_id INTEGER,
            item_id INTEGER,
            kind TEXT,
            active INTEGER
        );
        CREATE TABLE editors (
            workspace_id INTEGER,
            item_id INTEGER,
            path BLOB
        );
        CREATE TABLE editor_selections (
            workspace_id INTEGER,
            editor_id INTEGER,
            start INTEGER,
            end INTEGER
        );",
    )
    .unwrap();
    conn
}

#[test]
fn query_editors_basic() {
    let conn = create_test_db();
    conn.execute_batch(
        "INSERT INTO items VALUES (1, 10, 'Editor', 1);
         INSERT INTO editors VALUES (1, 10, X'2F746D702F666F6F2E7273');
         INSERT INTO editor_selections VALUES (1, 10, 42, 42);",
    )
    .unwrap();
    // X'2F746D702F666F6F2E7273' = "/tmp/foo.rs"

    let result = query_editors(&conn).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].path.to_str().unwrap(), "/tmp/foo.rs");
    assert_eq!(result[0].sel_start, Some(42));
    assert_eq!(result[0].sel_end, Some(42));
}

#[test]
fn query_editors_empty_db() {
    let conn = create_test_db();
    let result = query_editors(&conn).unwrap();
    assert!(result.is_empty());
}

#[test]
fn query_editors_inactive_items_excluded() {
    let conn = create_test_db();
    conn.execute_batch(
        "INSERT INTO items VALUES (1, 10, 'Editor', 0);
         INSERT INTO editors VALUES (1, 10, X'2F746D702F666F6F2E7273');",
    )
    .unwrap();

    let result = query_editors(&conn).unwrap();
    assert!(result.is_empty());
}

#[test]
fn query_editors_null_selections() {
    let conn = create_test_db();
    conn.execute_batch(
        "INSERT INTO items VALUES (1, 10, 'Editor', 1);
         INSERT INTO editors VALUES (1, 10, X'2F746D702F666F6F2E7273');",
    )
    .unwrap();
    // No editor_selections row — LEFT JOIN gives NULLs.

    let result = query_editors(&conn).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].sel_start, None);
    assert_eq!(result[0].sel_end, None);
}

#[test]
fn query_editors_multiple_files() {
    let conn = create_test_db();
    conn.execute_batch(
        "INSERT INTO items VALUES (1, 10, 'Editor', 1);
         INSERT INTO items VALUES (1, 20, 'Editor', 1);
         INSERT INTO editors VALUES (1, 10, X'2F746D702F612E7273');
         INSERT INTO editors VALUES (1, 20, X'2F746D702F622E7273');
         INSERT INTO editor_selections VALUES (1, 10, 0, 5);
         INSERT INTO editor_selections VALUES (1, 20, 10, 20);",
    )
    .unwrap();
    // X'2F746D702F612E7273' = "/tmp/a.rs"
    // X'2F746D702F622E7273' = "/tmp/b.rs"

    let result = query_editors(&conn).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].path.to_str().unwrap(), "/tmp/a.rs");
    assert_eq!(result[1].path.to_str().unwrap(), "/tmp/b.rs");
}

#[test]
fn query_editors_non_utf8_path() {
    let conn = create_test_db();
    // Insert a path with invalid UTF-8: 0xFF byte.
    conn.execute_batch(
        "INSERT INTO items VALUES (1, 10, 'Editor', 1);
         INSERT INTO editors VALUES (1, 10, X'2FFF2F666F6F');",
    )
    .unwrap();

    let result = query_editors(&conn).unwrap();
    assert_eq!(result.len(), 1);
    // Non-UTF8 path should result in an empty PathBuf.
    assert_eq!(result[0].path, std::path::PathBuf::new());
}

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
fn is_narration_keybinding_current_keys() {
    let flush = serde_json::json!({"bindings": {"cmd-:": ["task::Spawn", {"task_name": "Flush"}]}});
    let toggle =
        serde_json::json!({"bindings": {"cmd-;": ["task::Spawn", {"task_name": "Toggle"}]}});
    assert!(is_narration_keybinding(&flush));
    assert!(is_narration_keybinding(&toggle));
}

#[test]
fn is_narration_keybinding_rejects_multi_binding() {
    let multi = serde_json::json!({"bindings": {"cmd-:": ["task::Spawn"], "cmd-k": ["other"]}});
    assert!(!is_narration_keybinding(&multi));
}

#[test]
fn is_narration_keybinding_rejects_unrelated() {
    let other = serde_json::json!({"bindings": {"cmd-s": ["editor::Save"]}});
    assert!(!is_narration_keybinding(&other));
}
