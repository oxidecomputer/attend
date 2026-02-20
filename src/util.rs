//! Shared utility functions.

use std::{fs, io};

use camino::Utf8Path;
use chrono::Utc;
use serde::Serialize;

/// Write to a file atomically by writing to a temporary sibling first.
///
/// Creates `<path>.tmp`, calls the writer closure, then renames to `<path>`.
/// This prevents readers from seeing partially-written files.
pub(crate) fn atomic_write(
    path: &Utf8Path,
    f: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    match f(&mut file) {
        Ok(()) => fs::rename(&tmp, path),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Return the current UTC time as an ISO 8601 string (e.g. `2026-02-18T15:30:45Z`).
pub fn utc_now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Wrapper that adds a `timestamp` field to any serializable payload.
#[derive(Serialize)]
pub struct Timestamped<T: Serialize> {
    pub timestamp: String,
    #[serde(flatten)]
    pub inner: T,
}

impl<T: Serialize> Timestamped<T> {
    /// Wrap a payload with the current UTC timestamp.
    pub fn now(inner: T) -> Self {
        Self {
            timestamp: utc_now(),
            inner,
        }
    }
}
