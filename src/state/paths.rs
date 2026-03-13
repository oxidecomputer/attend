//! Filesystem path helpers for hook state, install metadata, and shared cache.

use std::{fs, io};

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::util::atomic_write;

use super::SessionId;
use super::cache::cache_dir;

/// Root directory for hook system shared state (listening, receive lock, cache).
pub fn hooks_dir() -> Option<Utf8PathBuf> {
    Some(cache_dir()?.join("hooks"))
}

/// Path to the file that identifies the currently attending session.
pub fn listening_path() -> Option<Utf8PathBuf> {
    Some(hooks_dir()?.join("listening"))
}

/// Read the session ID of the currently attending session, if any.
pub fn listening_session() -> Option<SessionId> {
    std::fs::read_to_string(listening_path()?)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(SessionId::from)
}

/// Path to the installed version/components file.
pub(crate) fn version_path() -> Option<Utf8PathBuf> {
    Some(cache_dir()?.join("version.json"))
}

/// Metadata about the most recent hook install.
#[derive(Serialize, Deserialize, Debug, Default)]
pub(crate) struct InstallMeta {
    pub version: String,
    pub agents: Vec<String>,
    pub editors: Vec<String>,
    #[serde(default)]
    pub browsers: Vec<String>,
    #[serde(default)]
    pub shells: Vec<String>,
    pub dev: bool,
    /// Project directories where hooks have been installed via `--project`.
    #[serde(default)]
    pub project_paths: Vec<Utf8PathBuf>,
}

/// Read the install metadata, if any.
pub(crate) fn installed_meta() -> Option<InstallMeta> {
    let path = version_path()?;
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Save install metadata after a successful hook install.
pub(crate) fn save_install_meta(meta: &InstallMeta) {
    let Some(path) = version_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent); // Best-effort: will fail at write if missing
    }
    if let Err(e) = atomic_write(&path, |file| {
        serde_json::to_writer_pretty(io::BufWriter::new(file), meta).map_err(io::Error::other)
    }) {
        tracing::warn!("Failed to save install metadata: {e}");
    }
}

/// Path to the shared ordering cache.
pub(super) fn shared_cache_path() -> Option<Utf8PathBuf> {
    Some(hooks_dir()?.join("latest.json"))
}

/// Check whether an anyhow error chain contains an `io::ErrorKind::NotFound`.
pub(super) fn is_not_found(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::NotFound)
    })
}
