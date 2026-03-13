//! Cache directory resolution and test overrides.

use std::sync::OnceLock;

use camino::Utf8PathBuf;

// Thread-local override for `cache_dir()`, used by tests to redirect
// all state I/O to a temp directory without process-global mutation.
#[cfg(test)]
thread_local! {
    static CACHE_DIR_OVERRIDE: std::cell::RefCell<Option<Utf8PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Set the thread-local cache directory override for tests.
#[cfg(test)]
pub(crate) fn set_cache_dir_override(dir: Option<Utf8PathBuf>) {
    CACHE_DIR_OVERRIDE.with(|cell| *cell.borrow_mut() = dir);
}

/// RAII guard that redirects `cache_dir()` to a temp directory for the
/// duration of a test. Clears the override on drop, even if the test panics.
#[cfg(test)]
pub(crate) struct CacheDirGuard {
    /// Holds the tempdir alive; dropped (and cleaned up) with the guard.
    _tmp: tempfile::TempDir,
    /// Exposed so tests can construct paths relative to the override.
    pub cache: Utf8PathBuf,
}

#[cfg(test)]
impl CacheDirGuard {
    /// Create a new temp directory and install it as the cache override.
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let cache = Utf8PathBuf::try_from(tmp.path().to_path_buf()).expect("non-UTF-8 temp dir");
        set_cache_dir_override(Some(cache.clone()));
        std::fs::create_dir_all(&cache).expect("failed to create cache dir");
        Self { _tmp: tmp, cache }
    }
}

#[cfg(test)]
impl Drop for CacheDirGuard {
    fn drop(&mut self) {
        set_cache_dir_override(None);
    }
}

/// Cached result of checking `ATTEND_CACHE_DIR`. Evaluated once per process.
static ENV_CACHE_DIR: OnceLock<Option<Utf8PathBuf>> = OnceLock::new();

/// Check the `ATTEND_CACHE_DIR` env var (once per process).
///
/// - Not set: returns `None` (fall through to platform default).
/// - Set to a path: uses that path directly.
/// - Set to empty string: auto-creates a random temp directory (useful for
///   manual testing and parallel runs). The temp dir is leaked intentionally
///   so it persists for the process lifetime.
fn env_cache_dir() -> Option<&'static Utf8PathBuf> {
    ENV_CACHE_DIR
        .get_or_init(|| {
            let val = std::env::var("ATTEND_CACHE_DIR").ok()?;
            if val.is_empty() {
                let tmp = tempfile::tempdir().expect("failed to create temp dir");
                let path =
                    Utf8PathBuf::try_from(tmp.path().to_path_buf()).expect("non-UTF-8 temp dir");
                // Leak: directory persists for process lifetime without cleanup.
                std::mem::forget(tmp);
                Some(path)
            } else {
                Some(Utf8PathBuf::from(val))
            }
        })
        .as_ref()
}

/// Return the platform cache directory for attend.
///
/// Resolution order:
/// 1. Thread-local override (unit tests via [`CacheDirGuard`]).
/// 2. `ATTEND_CACHE_DIR` env var (e2e test harness, manual testing).
/// 3. Platform default (`~/Library/Caches/attend` on macOS,
///    `$XDG_CACHE_HOME/attend` on Linux).
pub fn cache_dir() -> Option<Utf8PathBuf> {
    #[cfg(test)]
    {
        let override_dir = CACHE_DIR_OVERRIDE.with(|cell| cell.borrow().clone());
        if let Some(dir) = override_dir {
            return Some(dir);
        }
    }
    if let Some(dir) = env_cache_dir() {
        return Some(dir.clone());
    }
    let dir = dirs::cache_dir()?;
    let dir = Utf8PathBuf::try_from(dir).ok()?;
    Some(dir.join("attend"))
}
