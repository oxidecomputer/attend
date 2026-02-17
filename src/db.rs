use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

pub struct RawEditor {
    pub path: String,
    pub sel_start: Option<i64>,
    pub sel_end: Option<i64>,
}

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

pub fn query_editors(db_path: &Path) -> anyhow::Result<Vec<RawEditor>> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("failed to open DB")?;

    // Enable WAL mode reading
    conn.pragma_update(None, "journal_mode", "wal")
        .context("failed to set journal_mode")?;

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
            let path = String::from_utf8(path_bytes).unwrap_or_default();
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
