//! Shared utility functions.

use std::path::Path;
use std::{fs, io};

use chrono::Utc;
use serde::Serialize;

/// Write to a file atomically by writing to a temporary sibling first.
///
/// Creates `<path>.tmp`, calls the writer closure, then renames to `<path>`.
/// This prevents readers from seeing partially-written files.
pub(crate) fn atomic_write(
    path: impl AsRef<Path>,
    f: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let path = path.as_ref();
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

/// Atomically write string content to a file (convenience wrapper).
pub(crate) fn atomic_write_str(path: impl AsRef<Path>, content: &str) -> io::Result<()> {
    atomic_write(path, |f| io::Write::write_all(f, content.as_bytes()))
}

/// Atomically replace a directory's contents.
///
/// Writes files to a staging directory (`<dir>.staging`), removes the
/// old directory, and renames the staging directory into place. This
/// prevents readers from seeing a partially-written skill directory.
pub(crate) fn atomic_replace_dir(dir: impl AsRef<Path>, files: &[(&str, &str)]) -> io::Result<()> {
    let dir = dir.as_ref();
    let staging = dir.with_extension("staging");

    // Clean up any leftover staging directory from a prior crash.
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;

    for (name, content) in files {
        fs::write(staging.join(name), content)?;
    }

    // Swap: remove old dir, rename staging into place.
    let _ = fs::remove_dir_all(dir);
    fs::rename(&staging, dir)
}

/// Return the current UTC time as an ISO 8601 string (e.g. `2026-02-18T15:30:45Z`).
pub fn utc_now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Return the current UTC time with nanosecond precision
/// (e.g. `2026-02-18T15:30:45.123456789Z`).
///
/// Used for staging filenames where sub-second ordering matters.
pub fn utc_now_nanos() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.9fZ").to_string()
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
