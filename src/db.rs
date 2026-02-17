use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// A row from the editors + selections join, before offset resolution.
pub struct RawEditor {
    /// Absolute file path.
    pub path: PathBuf,
    /// Byte offset of selection start, if any.
    pub sel_start: Option<i64>,
    /// Byte offset of selection end, if any.
    pub sel_end: Option<i64>,
}

/// Raw editors and terminals returned from the Zed database.
pub struct QueryResult {
    /// Active editor tabs with byte-offset selections.
    pub editors: Vec<RawEditor>,
    /// Working directories of active terminal tabs.
    pub terminals: Vec<PathBuf>,
}

/// Find the most recently active Zed SQLite database.
pub fn find_zed_db() -> Option<PathBuf> {
    let data_dir = dirs::data_dir()?;
    let zed_db_dir = data_dir.join("Zed").join("db");

    let mut candidates: Vec<PathBuf> = fs::read_dir(&zed_db_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("0-")))
        .map(|e| e.path().join("db.sqlite"))
        .filter(|p| p.exists())
        .collect();

    // Pick the one with the most recently modified WAL
    candidates.sort_by(|a, b| {
        let wal_mtime = |p: &Path| {
            let wal = p.with_extension("sqlite-wal");
            fs::metadata(&wal).and_then(|m| m.modified()).ok()
        };
        wal_mtime(b).cmp(&wal_mtime(a))
    });

    candidates.into_iter().next()
}

/// Query active editors and terminals from the Zed database.
pub fn query(db_path: &Path) -> anyhow::Result<QueryResult> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("failed to open DB")?;

    let editors = query_editors(&conn)?;
    let terminals = query_terminals(&conn)?;

    Ok(QueryResult { editors, terminals })
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
            let path = PathBuf::from(String::from_utf8(path_bytes).unwrap_or_default());
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

/// Query working directories of active terminal tabs.
fn query_terminals(conn: &rusqlite::Connection) -> anyhow::Result<Vec<PathBuf>> {
    let mut stmt = conn
        .prepare(
            "SELECT t.working_directory_path \
             FROM items i \
             JOIN terminals t ON i.item_id = t.item_id AND i.workspace_id = t.workspace_id \
             WHERE i.kind = 'Terminal' AND i.active = 1",
        )
        .context("prepare failed")?;

    let terminals: Vec<PathBuf> = stmt
        .query_map([], |row| {
            let s: String = row.get(0)?;
            Ok(PathBuf::from(s))
        })
        .context("query failed")?
        .filter_map(|r| r.ok())
        .collect();

    Ok(terminals)
}
