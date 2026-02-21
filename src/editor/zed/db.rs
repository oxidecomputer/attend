//! Zed SQLite database queries for active editor state.

use std::fs;

use anyhow::Context;

use crate::editor::RawEditor;

/// Find the most recently active Zed SQLite database.
pub(super) fn find_db() -> Option<std::path::PathBuf> {
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

/// Query active editor tabs with their byte-offset selections.
pub(super) fn query_editors(conn: &rusqlite::Connection) -> anyhow::Result<Vec<RawEditor>> {
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
                    tracing::warn!("Skipping non-UTF-8 path from Zed DB: {e}");
                    return Ok(None);
                }
            };
            Ok(Some(RawEditor {
                path,
                sel_start: row.get(1)?,
                sel_end: row.get(2)?,
            }))
        })
        .context("query failed")?
        .filter_map(|r| r.ok())
        .flatten()
        .collect();

    Ok(editors)
}
