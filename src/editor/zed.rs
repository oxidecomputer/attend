use std::fs;
use std::path::PathBuf;

use anyhow::Context;

use super::{Editor, QueryResult, RawEditor};

/// Zed config directory (`~/.config/zed`).
///
/// Zed uses `~/.config/zed` on all platforms, not the platform-native
/// config directory (e.g. `~/Library/Application Support` on macOS).
fn zed_config_dir() -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".config").join("zed"))
}

/// Zed editor backend — queries the Zed SQLite database for open tabs.
pub struct Zed;

impl Editor for Zed {
    fn name(&self) -> &'static str {
        "zed"
    }

    fn query(&self) -> anyhow::Result<Option<QueryResult>> {
        query()
    }

    fn watch_paths(&self) -> Vec<std::path::PathBuf> {
        let Some(data) = dirs::data_dir() else {
            return Vec::new();
        };
        let db_dir = data.join("Zed").join("db");
        if db_dir.is_dir() {
            vec![db_dir]
        } else {
            Vec::new()
        }
    }

    fn install_dictation(&self, bin_cmd: &str) -> anyhow::Result<()> {
        install_task(bin_cmd)?;
        install_keybinding()?;
        println!("Installed Zed dictation task and keybinding.");
        Ok(())
    }

    fn uninstall_dictation(&self) -> anyhow::Result<()> {
        uninstall_task()?;
        uninstall_keybinding()?;
        println!("Removed Zed dictation task and keybinding.");
        Ok(())
    }
}

fn query() -> anyhow::Result<Option<QueryResult>> {
    let db_path = match find_db() {
        Some(p) => p,
        None => return Ok(None),
    };

    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("failed to open DB")?;

    let editors = query_editors(&conn)?;

    Ok(Some(QueryResult { editors }))
}

/// Find the most recently active Zed SQLite database.
fn find_db() -> Option<std::path::PathBuf> {
    let data_dir = dirs::data_dir()?;
    let zed_db_dir = data_dir.join("Zed").join("db");

    let candidates: Vec<std::path::PathBuf> = fs::read_dir(&zed_db_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("0-")))
        .map(|e| e.path().join("db.sqlite"))
        .filter(|p| p.exists())
        .collect();

    // Pick the one with the most recently modified WAL (precompute mtimes to
    // avoid repeated syscalls inside the sort comparator).
    let mut with_mtime: Vec<_> = candidates
        .into_iter()
        .map(|p| {
            let mtime = p
                .with_extension("sqlite-wal")
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok());
            (p, mtime)
        })
        .collect();
    with_mtime.sort_by(|(_, a), (_, b)| b.cmp(a));

    with_mtime.into_iter().next().map(|(p, _)| p)
}

/// Install the Zed task definition for toggling dictation.
///
/// Preserves existing file content (comments, formatting) by appending
/// via string manipulation rather than parse-and-rewrite.
fn install_task(bin_cmd: &str) -> anyhow::Result<()> {
    let tasks_path = zed_config_dir()?.join("tasks.json");

    let task_entry = serde_json::json!({
        "label": "Toggle Dictation",
        "command": bin_cmd,
        "args": ["dictate", "toggle"],
        "hide": "always",
        "reveal": "never",
        "allow_concurrent_runs": false,
        "use_new_terminal": true
    });

    if let Some(parent) = tasks_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if !tasks_path.exists() {
        let mut output = serde_json::to_string_pretty(&[&task_entry])?;
        output.push('\n');
        fs::write(&tasks_path, output)?;
        return Ok(());
    }

    let content = fs::read_to_string(&tasks_path)?;

    let existing: Vec<serde_json::Value> =
        parse_jsonc(&content).unwrap_or_default();

    let prev = existing.iter().find(|t| {
        t.get("label")
            .and_then(|l| l.as_str())
            .is_some_and(|l| l == "Toggle Dictation")
    });

    if prev.is_some_and(|p| *p == task_entry) {
        // Already up to date.
        return Ok(());
    }

    if prev.is_some() {
        // Replace outdated entry (rewrite unavoidable).
        let mut tasks = existing;
        tasks.retain(|t| {
            t.get("label")
                .and_then(|l| l.as_str())
                .is_none_or(|l| l != "Toggle Dictation")
        });
        tasks.push(task_entry);
        let mut output = serde_json::to_string_pretty(&tasks)?;
        output.push('\n');
        fs::write(&tasks_path, output)?;
        return Ok(());
    }

    // Append to existing file, preserving comments and formatting.
    let new_json = serde_json::to_string_pretty(&task_entry)?;
    let output = append_to_json_array(&content, &new_json)?;
    fs::write(&tasks_path, output)?;

    Ok(())
}

/// Remove the Zed task definition for dictation.
fn uninstall_task() -> anyhow::Result<()> {
    let tasks_path = zed_config_dir()?.join("tasks.json");

    if !tasks_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&tasks_path)?;
    let mut tasks: Vec<serde_json::Value> =
        parse_jsonc(&content).unwrap_or_default();

    let before = tasks.len();
    tasks.retain(|t| {
        t.get("label")
            .and_then(|l| l.as_str())
            .is_none_or(|l| l != "Toggle Dictation")
    });

    if tasks.len() < before {
        let mut output = serde_json::to_string_pretty(&tasks)?;
        output.push('\n');
        fs::write(&tasks_path, output)?;
    }

    Ok(())
}

/// Remove the Zed keybinding for dictation.
fn uninstall_keybinding() -> anyhow::Result<()> {
    let keymap_path = zed_config_dir()?.join("keymap.json");

    if !keymap_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&keymap_path)?;
    let mut keymap: Vec<serde_json::Value> =
        parse_jsonc(&content).unwrap_or_default();

    let before = keymap.len();
    keymap.retain(|entry| !is_dictation_keybinding(entry));

    if keymap.len() < before {
        let mut output = serde_json::to_string_pretty(&keymap)?;
        output.push('\n');
        fs::write(&keymap_path, output)?;
    }

    Ok(())
}

/// Install a Zed keybinding for the Toggle Dictation task.
///
/// Preserves existing file content (comments, formatting) by appending
/// via string manipulation rather than parse-and-rewrite.
fn install_keybinding() -> anyhow::Result<()> {
    let keymap_path = zed_config_dir()?.join("keymap.json");

    let binding_entry = serde_json::json!({
        "bindings": {
            "cmd-:": ["task::Spawn", {"task_name": "Toggle Dictation"}]
        }
    });

    if let Some(parent) = keymap_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if !keymap_path.exists() {
        let mut output = serde_json::to_string_pretty(&[&binding_entry])?;
        output.push('\n');
        fs::write(&keymap_path, output)?;
        return Ok(());
    }

    let content = fs::read_to_string(&keymap_path)?;

    // Check if already installed.
    let existing: Vec<serde_json::Value> =
        parse_jsonc(&content).unwrap_or_default();
    if existing.iter().any(|e| is_dictation_keybinding(e)) {
        return Ok(());
    }

    // Append to existing file, preserving comments and formatting.
    let new_json = serde_json::to_string_pretty(&binding_entry)?;
    let output = append_to_json_array(&content, &new_json)?;
    fs::write(&keymap_path, output)?;

    Ok(())
}

/// Append a JSON object to a top-level JSON array by string manipulation.
///
/// Finds the last `]` in the file and inserts a comma + the new entry
/// before it, preserving all existing content (comments, whitespace).
fn append_to_json_array(content: &str, new_entry_json: &str) -> anyhow::Result<String> {
    // Find the last `]` (closing the top-level array).
    let close_bracket = content
        .rfind(']')
        .ok_or_else(|| anyhow::anyhow!("no closing ] found in JSON array"))?;

    // Check if the array has existing entries (look for non-whitespace before `]`).
    let before = &content[..close_bracket];
    let needs_comma = before
        .bytes()
        .rev()
        .find(|&b| !b.is_ascii_whitespace())
        .is_some_and(|b| b != b'[' && b != b',');

    let mut out = String::with_capacity(content.len() + new_entry_json.len() + 16);
    out.push_str(before.trim_end());
    if needs_comma {
        out.push(',');
    }
    out.push('\n');

    // Indent the new entry to match typical Zed formatting (2 spaces).
    for line in new_entry_json.lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }

    // Close the array and preserve any trailing content (newline).
    out.push(']');
    let after = &content[close_bracket + 1..];
    if after.is_empty() {
        out.push('\n');
    } else {
        out.push_str(after);
    }

    Ok(out)
}

/// Check whether a keymap entry is solely our dictation keybinding.
fn is_dictation_keybinding(entry: &serde_json::Value) -> bool {
    let Some(bindings) = entry.get("bindings").and_then(|b| b.as_object()) else {
        return false;
    };
    if bindings.len() != 1 {
        return false;
    }
    // Match current key or legacy key.
    let is_spawn = |key: &str| {
        bindings
            .get(key)
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.first().and_then(|s| s.as_str()) == Some("task::Spawn"))
    };
    is_spawn("cmd-:") || is_spawn("ctrl-shift-'") || is_spawn("cmd-shift-'") || is_spawn("cmd-shift-d")
}

/// Parse a JSONC string (Zed's config format: `//` comments + trailing commas).
fn parse_jsonc<T: serde::de::DeserializeOwned>(input: &str) -> serde_json::Result<T> {
    let stripped = strip_json_comments(input);
    let clean = strip_trailing_commas(&stripped);
    serde_json::from_str(&clean)
}

/// Strip `//` line comments from JSON content (Zed supports comments in JSON).
fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        if let Some(idx) = find_line_comment(line) {
            out.push_str(&line[..idx]);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Strip trailing commas before `]` and `}` (Zed allows trailing commas in JSONC).
fn strip_trailing_commas(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut in_string = false;
    let mut escaped = false;

    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            out.push(bytes[i]);
            continue;
        }
        match bytes[i] {
            b'\\' if in_string => {
                escaped = true;
                out.push(bytes[i]);
            }
            b'"' => {
                in_string = !in_string;
                out.push(bytes[i]);
            }
            b',' if !in_string => {
                // Look ahead past whitespace for ] or }
                let rest = &bytes[i + 1..];
                let next = rest.iter().find(|&&b| !b.is_ascii_whitespace());
                if next.is_some_and(|&b| b == b']' || b == b'}') {
                    continue; // skip trailing comma
                }
                out.push(bytes[i]);
            }
            _ => {
                out.push(bytes[i]);
            }
        }
    }

    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

/// Find the position of a `//` comment that's not inside a JSON string.
fn find_line_comment(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escaped = false;
    let bytes = line.as_bytes();

    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            continue;
        }
        match bytes[i] {
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'/' if !in_string && i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// Query active editor tabs with their byte-offset selections.
fn query_editors(conn: &rusqlite::Connection) -> anyhow::Result<Vec<RawEditor>> {
    let mut stmt = conn
        .prepare(
            "SELECT e.path, es.start, es.end \
             FROM items i \
             JOIN editors e ON i.item_id = e.item_id AND i.workspace_id = e.workspace_id \
             LEFT JOIN editor_selections es \
               ON e.item_id = es.editor_id AND e.workspace_id = es.workspace_id \
             WHERE i.kind = 'Editor' AND i.active = 1 \
             ORDER BY e.path, es.start",
        )
        .context("prepare failed")?;

    let editors: Vec<RawEditor> = stmt
        .query_map([], |row| {
            let path_bytes: Vec<u8> = row.get(0)?;
            let path = std::path::PathBuf::from(String::from_utf8(path_bytes).unwrap_or_default());
            Ok(RawEditor {
                path,
                sel_start: row.get(1)?,
                sel_end: row.get(2)?,
            })
        })
        .context("query failed")?
        .filter_map(|r| r.ok())
        .collect();

    Ok(editors)
}

#[cfg(test)]
mod tests {
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
    fn append_to_json_array_preserves_content() {
        let existing = "// Zed keymap\n[\n  {\n    \"context\": \"Editor\",\n    \"bindings\": {\n      \"alt-q\": \"vim::Rewrap\"\n    }\n  }\n]\n";
        let new_entry = "{\n  \"bindings\": {\n    \"cmd-:\": [\"task::Spawn\"]\n  }\n}";
        let result = append_to_json_array(existing, new_entry).unwrap();
        // Original comment and content preserved
        assert!(result.starts_with("// Zed keymap\n"));
        assert!(result.contains("\"alt-q\": \"vim::Rewrap\""));
        // New entry appended
        assert!(result.contains("\"cmd-:\""));
        // Valid JSON after stripping comments
        let parsed: Vec<serde_json::Value> =
            parse_jsonc(&result).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn append_to_array_with_trailing_comma() {
        let existing = "[\n  {\"a\": 1},\n]\n";
        let new_entry = "{\n  \"b\": 2\n}";
        let result = append_to_json_array(existing, new_entry).unwrap();
        // No double comma
        assert!(!result.contains(",,"));
        let parsed: Vec<serde_json::Value> = parse_jsonc(&result).unwrap();
        assert_eq!(parsed.len(), 2);
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
    fn append_to_empty_array() {
        let existing = "[]\n";
        let new_entry = "{\n  \"label\": \"test\"\n}";
        let result = append_to_json_array(existing, new_entry).unwrap();
        let parsed: Vec<serde_json::Value> =
            parse_jsonc(&result).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["label"], "test");
    }
}
