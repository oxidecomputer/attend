use super::db::query_editors;
use super::jsonc::{JsoncArray, parse_jsonc};
use super::keybindings::is_narration_keybinding;

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

/// A single active editor with a selection is returned correctly.
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

/// An empty database returns no editors.
#[test]
fn query_editors_empty_db() {
    let conn = create_test_db();
    let result = query_editors(&conn).unwrap();
    assert!(result.is_empty());
}

/// Inactive items (active=0) are excluded from results.
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

/// Editors without selection rows have None for sel_start/sel_end.
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

/// Multiple active editors are all returned.
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

/// Non-UTF-8 byte paths result in an empty PathBuf.
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

/// parse_jsonc handles full-line and inline comments.
#[test]
fn parse_jsonc_strips_comments() {
    let input = "// header\n{\"key\": \"value\"} // inline\n";
    let v: serde_json::Value = parse_jsonc(input).unwrap();
    assert_eq!(v["key"], "value");
}

/// URLs inside strings are preserved (not treated as comments).
#[test]
fn parse_jsonc_preserves_urls_in_strings() {
    let input = "{\"url\": \"https://example.com\"}";
    let v: serde_json::Value = parse_jsonc(input).unwrap();
    assert_eq!(v["url"], "https://example.com");
}

/// Trailing commas in arrays and objects are accepted.
#[test]
fn parse_jsonc_trailing_commas() {
    let input = "[{\"a\": 1}, {\"b\": 2},]";
    let parsed: Vec<serde_json::Value> = parse_jsonc(input).unwrap();
    assert_eq!(parsed.len(), 2);
}

/// Trailing commas in objects are accepted.
#[test]
fn parse_jsonc_trailing_comma_in_object() {
    let input = "{\"a\": 1, \"b\": 2,}";
    let v: serde_json::Value = parse_jsonc(input).unwrap();
    assert_eq!(v["a"], 1);
    assert_eq!(v["b"], 2);
}

/// Combined: comments + trailing commas (realistic Zed config).
#[test]
fn parse_jsonc_with_comments_and_trailing_commas() {
    let input = "// Zed config\n[\n  {\"a\": 1},\n  // second entry\n  {\"b\": 2},\n]\n";
    let parsed: Vec<serde_json::Value> = parse_jsonc(input).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0]["a"], 1);
    assert_eq!(parsed[1]["b"], 2);
}

/// JsoncArray preserves comments across retain + push + save.
#[test]
fn jsonc_array_preserves_comments() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.json");
    std::fs::write(
        &path,
        "// My config\n[\n  // first entry\n  {\"a\": 1},\n  {\"b\": 2}\n]\n",
    )
    .unwrap();

    let mut arr = JsoncArray::open(&path).unwrap();
    assert_eq!(arr.elements().len(), 2);

    // Remove second element, add a third.
    arr.retain(|v| v.get("b").is_none());
    arr.push(serde_json::json!({"c": 3}));
    arr.save().unwrap();

    let output = std::fs::read_to_string(&path).unwrap();
    // Comments from the original file must survive.
    assert!(output.contains("// My config"), "header comment lost");
    assert!(output.contains("// first entry"), "inline comment lost");
    // New element must be present.
    assert!(output.contains("\"c\""));
    // Removed element must be gone.
    assert!(!output.contains("\"b\""));
}

/// Both flush (cmd-:) and toggle (cmd-;) keybindings are recognized.
#[test]
fn is_narration_keybinding_current_keys() {
    let flush = serde_json::json!({"bindings": {"cmd-:": ["task::Spawn", {"task_name": "Flush"}]}});
    let toggle =
        serde_json::json!({"bindings": {"cmd-;": ["task::Spawn", {"task_name": "Toggle"}]}});
    assert!(is_narration_keybinding(&flush));
    assert!(is_narration_keybinding(&toggle));
}

/// Keymaps with multiple bindings are rejected.
#[test]
fn is_narration_keybinding_rejects_multi_binding() {
    let multi = serde_json::json!({"bindings": {"cmd-:": ["task::Spawn"], "cmd-k": ["other"]}});
    assert!(!is_narration_keybinding(&multi));
}

/// Unrelated keybindings are rejected.
#[test]
fn is_narration_keybinding_rejects_unrelated() {
    let other = serde_json::json!({"bindings": {"cmd-s": ["editor::Save"]}});
    assert!(!is_narration_keybinding(&other));
}

// -- JsoncArray tests --

/// Opening a nonexistent file creates an empty array.
#[test]
fn jsonc_array_open_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent.json");
    let arr = JsoncArray::open(&path).unwrap();
    assert!(arr.elements().is_empty());
}

/// Push and save writes elements to disk.
#[test]
fn jsonc_array_push_and_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.json");

    let mut arr = JsoncArray::open(&path).unwrap();
    arr.push(serde_json::json!({"label": "task1"}));
    arr.push(serde_json::json!({"label": "task2"}));
    arr.save().unwrap();

    let arr2 = JsoncArray::open(&path).unwrap();
    assert_eq!(arr2.elements().len(), 2);
    assert_eq!(arr2.elements()[0]["label"], "task1");
    assert_eq!(arr2.elements()[1]["label"], "task2");
}

/// Retain removes matching elements and preserves others.
#[test]
fn jsonc_array_retain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.json");
    std::fs::write(&path, "[{\"a\": 1}, {\"b\": 2}, {\"a\": 3}]\n").unwrap();

    let mut arr = JsoncArray::open(&path).unwrap();
    let removed = arr.retain(|v| v.get("a").is_none());
    assert_eq!(removed, 2);
    assert!(arr.is_modified());
    arr.save().unwrap();

    let arr2 = JsoncArray::open(&path).unwrap();
    assert_eq!(arr2.elements().len(), 1);
    assert_eq!(arr2.elements()[0]["b"], 2);
}

/// Retain on empty array returns 0 and does not modify.
#[test]
fn jsonc_array_retain_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.json");

    let mut arr = JsoncArray::open(&path).unwrap();
    let removed = arr.retain(|_| false);
    assert_eq!(removed, 0);
    assert!(!arr.is_modified());
}

/// JsoncArray handles files with comments and trailing commas.
#[test]
fn jsonc_array_with_comments() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.json");
    std::fs::write(
        &path,
        "// header comment\n[\n  // item comment\n  {\"a\": 1},\n]\n",
    )
    .unwrap();

    let arr = JsoncArray::open(&path).unwrap();
    assert_eq!(arr.elements().len(), 1);
    assert_eq!(arr.elements()[0]["a"], 1);
}

/// parse_jsonc returns null-equivalent for empty input.
#[test]
fn parse_jsonc_empty_input() {
    let result: serde_json::Value = parse_jsonc("").unwrap();
    assert!(result.is_null());
}

/// parse_jsonc rejects invalid JSONC.
#[test]
fn parse_jsonc_invalid_input() {
    let result: Result<serde_json::Value, _> = parse_jsonc("not json at all {{{");
    assert!(result.is_err());
}

// -- Keybinding install logic tests --

/// install_keybinding adds a keybinding to an empty keymap.
#[test]
fn keybinding_install_empty_keymap() {
    let dir = tempfile::tempdir().unwrap();
    let keymap_path = dir.path().join("keymap.json");

    // Simulate install_keybinding logic on a temp file
    let mut keymap = JsoncArray::open(&keymap_path).unwrap();
    let mut bindings = serde_json::Map::new();
    bindings.insert(
        "cmd-;".to_string(),
        serde_json::json!(["task::Spawn", {"task_name": "Toggle"}]),
    );
    keymap.push(serde_json::json!({ "bindings": bindings }));
    keymap.save().unwrap();

    let keymap2 = JsoncArray::open(&keymap_path).unwrap();
    assert_eq!(keymap2.elements().len(), 1);
    assert!(is_narration_keybinding(&keymap2.elements()[0]));
}

/// Uninstall keybinding removes narration entries but keeps others.
#[test]
fn keybinding_uninstall_preserves_others() {
    let dir = tempfile::tempdir().unwrap();
    let keymap_path = dir.path().join("keymap.json");

    // Write a keymap with a narration binding and a non-narration binding
    let content = serde_json::json!([
        {"bindings": {"cmd-;": ["task::Spawn", {"task_name": "Toggle"}]}},
        {"bindings": {"cmd-s": ["editor::Save"]}}
    ]);
    std::fs::write(&keymap_path, serde_json::to_string(&content).unwrap()).unwrap();

    let mut keymap = JsoncArray::open(&keymap_path).unwrap();
    keymap.retain(|entry| !is_narration_keybinding(entry));
    keymap.save().unwrap();

    let keymap2 = JsoncArray::open(&keymap_path).unwrap();
    assert_eq!(keymap2.elements().len(), 1);
    assert_eq!(
        keymap2.elements()[0]["bindings"]["cmd-s"][0],
        "editor::Save"
    );
}

// -- Task install logic tests --

/// Install task adds a task to an empty tasks file.
#[test]
fn task_install_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let tasks_path = dir.path().join("tasks.json");

    let mut tasks = JsoncArray::open(&tasks_path).unwrap();
    tasks.push(serde_json::json!({
        "label": "Flush",
        "command": "attend",
        "args": ["narrate", "flush"],
        "hide": "always"
    }));
    tasks.save().unwrap();

    let tasks2 = JsoncArray::open(&tasks_path).unwrap();
    assert_eq!(tasks2.elements().len(), 1);
    assert_eq!(tasks2.elements()[0]["label"], "Flush");
}

/// Uninstall task removes narration tasks but keeps others.
#[test]
fn task_uninstall_preserves_others() {
    let dir = tempfile::tempdir().unwrap();
    let tasks_path = dir.path().join("tasks.json");

    let content = serde_json::json!([
        {"label": "Flush", "command": "attend"},
        {"label": "Build", "command": "cargo build"}
    ]);
    std::fs::write(&tasks_path, serde_json::to_string(&content).unwrap()).unwrap();

    let mut tasks = JsoncArray::open(&tasks_path).unwrap();
    tasks.retain(|t| {
        let label = t.get("label").and_then(|l| l.as_str());
        label != Some("Flush")
    });
    tasks.save().unwrap();

    let tasks2 = JsoncArray::open(&tasks_path).unwrap();
    assert_eq!(tasks2.elements().len(), 1);
    assert_eq!(tasks2.elements()[0]["label"], "Build");
}

/// Task replace: removing old entry with same label then adding new one.
#[test]
fn task_install_replaces_existing() {
    let dir = tempfile::tempdir().unwrap();
    let tasks_path = dir.path().join("tasks.json");

    let content = serde_json::json!([
        {"label": "Flush", "command": "/old/path/attend", "args": ["narrate", "flush"]}
    ]);
    std::fs::write(&tasks_path, serde_json::to_string(&content).unwrap()).unwrap();

    let mut tasks = JsoncArray::open(&tasks_path).unwrap();
    tasks.retain(|t| t.get("label").and_then(|l| l.as_str()) != Some("Flush"));
    tasks.push(serde_json::json!({
        "label": "Flush",
        "command": "/new/path/attend",
        "args": ["narrate", "flush"]
    }));
    tasks.save().unwrap();

    let tasks2 = JsoncArray::open(&tasks_path).unwrap();
    assert_eq!(tasks2.elements().len(), 1);
    assert_eq!(tasks2.elements()[0]["command"], "/new/path/attend");
}
