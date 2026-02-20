//! Shared utility functions.

use std::{fs, io};

use camino::Utf8Path;
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
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) };
    let time = tv.tv_sec;
    let tm = unsafe { *libc::gmtime(&time) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    )
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
