use std::fs;
use std::path::PathBuf;

use anyhow::Context;

use super::{Editor, QueryResult, RawEditor};

/// Zed config directory (`~/.config/zed`).
///
/// Zed uses `~/.config/zed` on all platforms, not the platform-native
/// config directory (e.g. `~/Library/Application Support` on macOS).
fn zed_config_dir() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
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

    fn install_narration(&self, bin_cmd: &str) -> anyhow::Result<()> {
        install_task(bin_cmd, "attend: toggle narration", &["narrate", "toggle"])?;
        install_task(bin_cmd, "attend: start narration", &["narrate", "start"])?;
        install_keybinding("cmd-;", "attend: toggle narration")?;
        install_keybinding("cmd-:", "attend: start narration")?;
        println!("Installed Zed narration tasks and keybindings.");
        Ok(())
    }

    fn uninstall_narration(&self) -> anyhow::Result<()> {
        uninstall_task()?;
        uninstall_keybinding()?;
        println!("Removed Zed narration task and keybinding.");
        Ok(())
    }

    fn check_narration(&self) -> anyhow::Result<Vec<String>> {
        check_narration_health()
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

/// Known narration keybindings (current + legacy).
const NARRATION_KEYS: &[&str] = &[
    "cmd-:", // start (current)
    "cmd-;", // toggle (current)
];

/// Narration task labels.
const NARRATION_TASK_LABELS: &[&str] = &["attend: toggle narration", "attend: start narration"];

/// Legacy task labels from previous versions.
const LEGACY_TASK_LABELS: &[&str] = &[
    "Toggle Dictation",
    "Flush Dictation",
    "Toggle Narration",
    "Flush Narration",
    "attend: flush narration",
];

/// Read a Zed JSONC config file as a JSON array, or empty vec if missing/invalid.
fn read_jsonc_array(path: &std::path::Path) -> Vec<serde_json::Value> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match parse_jsonc(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(path = %path.display(), "Failed to parse JSONC: {e}");
            Vec::new()
        }
    }
}

/// Write a JSON array to a config file with pretty formatting.
fn write_json_array(path: &std::path::Path, items: &[serde_json::Value]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = serde_json::to_string_pretty(items)?;
    output.push('\n');
    fs::write(path, output)?;
    Ok(())
}

/// Install a Zed task definition for narration.
fn install_task(bin_cmd: &str, label: &str, args: &[&str]) -> anyhow::Result<()> {
    let tasks_path = zed_config_dir()?.join("tasks.json");
    let mut tasks = read_jsonc_array(&tasks_path);

    let task_entry = serde_json::json!({
        "label": label,
        "command": bin_cmd,
        "args": args,
        "hide": "always",
        "reveal": "never",
        "allow_concurrent_runs": false,
        "use_new_terminal": true
    });

    if tasks.contains(&task_entry) {
        return Ok(());
    }

    // Warn about stale command path before replacing.
    if let Some(cmd) = tasks
        .iter()
        .find(|t| t.get("label").and_then(|l| l.as_str()) == Some(label))
        .and_then(|t| t.get("command").and_then(|c| c.as_str()))
        && !std::path::Path::new(cmd).exists()
    {
        tracing::warn!("Replacing stale command path: {cmd}");
    }

    // Remove both current and legacy labels
    tasks.retain(|t| {
        let l = t.get("label").and_then(|l| l.as_str());
        l != Some(label) && !l.is_some_and(|l| LEGACY_TASK_LABELS.contains(&l))
    });
    tasks.push(task_entry);
    write_json_array(&tasks_path, &tasks)
}

/// Remove the Zed task definitions for narration (current + legacy).
fn uninstall_task() -> anyhow::Result<()> {
    let tasks_path = zed_config_dir()?.join("tasks.json");
    let mut tasks = read_jsonc_array(&tasks_path);

    let before = tasks.len();
    tasks.retain(|t| {
        let label = t.get("label").and_then(|l| l.as_str());
        !label
            .is_some_and(|l| NARRATION_TASK_LABELS.contains(&l) || LEGACY_TASK_LABELS.contains(&l))
    });

    if tasks.len() < before {
        write_json_array(&tasks_path, &tasks)?;
    }
    Ok(())
}

/// Remove the Zed keybindings for narration.
fn uninstall_keybinding() -> anyhow::Result<()> {
    let keymap_path = zed_config_dir()?.join("keymap.json");
    let mut keymap = read_jsonc_array(&keymap_path);

    let before = keymap.len();
    keymap.retain(|entry| !is_narration_keybinding(entry));

    if keymap.len() < before {
        write_json_array(&keymap_path, &keymap)?;
    }
    Ok(())
}

/// Install a Zed keybinding for a narration task.
///
/// Skips installation if the task is already bound to any key (user may have
/// reassigned it) or if the default key is already bound to something else.
fn install_keybinding(key: &str, task_name: &str) -> anyhow::Result<()> {
    let keymap_path = zed_config_dir()?.join("keymap.json");
    let mut keymap = read_jsonc_array(&keymap_path);

    let task_already_bound = keymap.iter().any(|e| {
        e.get("bindings")
            .and_then(|b| b.as_object())
            .is_some_and(|b| {
                b.values().any(|v| {
                    v.as_array().is_some_and(|a| {
                        a.first().and_then(|s| s.as_str()) == Some("task::Spawn")
                            && a.get(1)
                                .and_then(|o| o.get("task_name"))
                                .and_then(|n| n.as_str())
                                == Some(task_name)
                    })
                })
            })
    });
    if task_already_bound {
        return Ok(());
    }

    let key_already_bound = keymap.iter().any(|e| {
        e.get("bindings")
            .and_then(|b| b.as_object())
            .is_some_and(|b| b.contains_key(key))
    });
    if key_already_bound {
        return Ok(());
    }

    let mut bindings = serde_json::Map::new();
    bindings.insert(
        key.to_string(),
        serde_json::json!(["task::Spawn", {"task_name": task_name}]),
    );
    keymap.push(serde_json::json!({ "bindings": bindings }));
    write_json_array(&keymap_path, &keymap)
}

/// Check whether a keymap entry is solely our narration keybinding.
fn is_narration_keybinding(entry: &serde_json::Value) -> bool {
    let Some(bindings) = entry.get("bindings").and_then(|b| b.as_object()) else {
        return false;
    };
    if bindings.len() != 1 {
        return false;
    }
    NARRATION_KEYS.iter().any(|key| {
        bindings
            .get(*key)
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.first().and_then(|s| s.as_str()) == Some("task::Spawn"))
    })
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

/// Check whether a command exists — either as an absolute path or on PATH.
fn command_exists(cmd: &str) -> bool {
    let path = std::path::Path::new(cmd);
    if path.is_absolute() {
        path.exists()
    } else {
        which::which(cmd).is_ok()
    }
}

/// Check health of installed Zed narration integration.
fn check_narration_health() -> anyhow::Result<Vec<String>> {
    let reinstall = "run `attend install --editor zed`";
    let mut warnings = Vec::new();

    // Check tasks
    let tasks_path = zed_config_dir()?.join("tasks.json");
    let tasks = read_jsonc_array(&tasks_path);

    if tasks.is_empty() && !tasks_path.exists() {
        warnings.push(format!("tasks.json not found: {reinstall}"));
    } else {
        for label in NARRATION_TASK_LABELS {
            let task = tasks
                .iter()
                .find(|t| t.get("label").and_then(|l| l.as_str()) == Some(label));
            match task {
                None => warnings.push(format!("{label} task not found: {reinstall}")),
                Some(t) => {
                    if let Some(cmd) = t.get("command").and_then(|c| c.as_str())
                        && !command_exists(cmd)
                    {
                        warnings.push(format!(
                            "task command path does not exist: {cmd}: reinstall with {reinstall}"
                        ));
                    }
                }
            }
        }
    }

    // Check keybindings
    let keymap_path = zed_config_dir()?.join("keymap.json");
    let keymap = read_jsonc_array(&keymap_path);

    if keymap.is_empty() && !keymap_path.exists() {
        warnings.push(format!("keymap.json not found: {reinstall}"));
    } else if !keymap.iter().any(is_narration_keybinding) {
        warnings.push(format!("narration keybinding not found: {reinstall}"));
    }

    Ok(warnings)
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
            let path = match String::from_utf8(path_bytes) {
                Ok(s) => std::path::PathBuf::from(s),
                Err(e) => {
                    tracing::warn!("Skipping non-UTF8 path from Zed DB: {e}");
                    return Ok(RawEditor {
                        path: std::path::PathBuf::new(),
                        sel_start: None,
                        sel_end: None,
                    });
                }
            };
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
mod tests;
