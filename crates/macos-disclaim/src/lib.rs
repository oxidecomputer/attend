//! Spawn a child process with macOS TCC responsibility disclaimed.
//!
//! Uses the private `responsibility_spawnattrs_setdisclaim` API to break
//! the TCC responsible-process chain, causing microphone and accessibility
//! permissions to accrue to the spawned binary rather than the parent.
//!
//! This API is used by LLDB, Firefox, Chromium, and Qt Creator. It is not
//! in the public macOS SDK headers but is available at runtime via
//! `libresponsibility.dylib`.

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

use libc::{c_char, c_int, c_short, pid_t};

/// Resolve `responsibility_spawnattrs_setdisclaim` at runtime via `dlsym`.
///
/// Returns `None` if the symbol is unavailable (hypothetical future macOS
/// removal). Callers should fall back to a normal spawn.
fn resolve_disclaim_fn()
-> Option<unsafe extern "C" fn(*mut libc::posix_spawnattr_t, c_int) -> c_int> {
    // SAFETY: RTLD_DEFAULT searches all loaded images. The symbol name is a
    // static null-terminated byte string. transmute converts the void pointer
    // to a function pointer matching the known signature.
    unsafe {
        let sym = libc::dlsym(
            libc::RTLD_DEFAULT,
            c"responsibility_spawnattrs_setdisclaim".as_ptr(),
        );
        if sym.is_null() {
            None
        } else {
            Some(std::mem::transmute::<
                *mut libc::c_void,
                unsafe extern "C" fn(*mut libc::posix_spawnattr_t, c_int) -> c_int,
            >(sym))
        }
    }
}

/// RAII guard for `posix_spawnattr_t`. Calls `destroy` on drop.
struct SpawnAttrs {
    inner: libc::posix_spawnattr_t,
}

impl SpawnAttrs {
    fn new() -> io::Result<Self> {
        let mut inner: libc::posix_spawnattr_t = std::ptr::null_mut();
        // SAFETY: passing a valid mutable pointer to init.
        let ret = unsafe { libc::posix_spawnattr_init(&mut inner) };
        if ret != 0 {
            return Err(io::Error::from_raw_os_error(ret));
        }
        Ok(Self { inner })
    }

    fn as_mut_ptr(&mut self) -> *mut libc::posix_spawnattr_t {
        &mut self.inner
    }

    fn as_ptr(&self) -> *const libc::posix_spawnattr_t {
        &self.inner
    }

    fn set_flags(&mut self, flags: c_short) -> io::Result<()> {
        // SAFETY: self.inner was successfully initialized.
        let ret = unsafe { libc::posix_spawnattr_setflags(&mut self.inner, flags) };
        if ret != 0 {
            return Err(io::Error::from_raw_os_error(ret));
        }
        Ok(())
    }

    fn set_pgroup(&mut self, pgroup: pid_t) -> io::Result<()> {
        // SAFETY: self.inner was successfully initialized.
        let ret = unsafe { libc::posix_spawnattr_setpgroup(&mut self.inner, pgroup) };
        if ret != 0 {
            return Err(io::Error::from_raw_os_error(ret));
        }
        Ok(())
    }
}

impl Drop for SpawnAttrs {
    fn drop(&mut self) {
        // SAFETY: self.inner was successfully initialized and is being destroyed once.
        unsafe {
            libc::posix_spawnattr_destroy(&mut self.inner);
        }
    }
}

/// RAII guard for `posix_spawn_file_actions_t`. Calls `destroy` on drop.
struct FileActions {
    inner: libc::posix_spawn_file_actions_t,
}

impl FileActions {
    fn new() -> io::Result<Self> {
        let mut inner: libc::posix_spawn_file_actions_t = std::ptr::null_mut();
        // SAFETY: passing a valid mutable pointer to init.
        let ret = unsafe { libc::posix_spawn_file_actions_init(&mut inner) };
        if ret != 0 {
            return Err(io::Error::from_raw_os_error(ret));
        }
        Ok(Self { inner })
    }

    fn as_ptr(&self) -> *const libc::posix_spawn_file_actions_t {
        &self.inner
    }

    /// Redirect `fd` to the given open file path with the specified flags.
    fn add_open(
        &mut self,
        fd: c_int,
        path: &CStr,
        flags: c_int,
        mode: libc::mode_t,
    ) -> io::Result<()> {
        // SAFETY: self.inner was successfully initialized, path is a valid CStr.
        let ret = unsafe {
            libc::posix_spawn_file_actions_addopen(&mut self.inner, fd, path.as_ptr(), flags, mode)
        };
        if ret != 0 {
            return Err(io::Error::from_raw_os_error(ret));
        }
        Ok(())
    }

    /// Duplicate `from_fd` onto `to_fd` in the child.
    fn add_dup2(&mut self, from_fd: c_int, to_fd: c_int) -> io::Result<()> {
        // SAFETY: self.inner was successfully initialized.
        let ret =
            unsafe { libc::posix_spawn_file_actions_adddup2(&mut self.inner, from_fd, to_fd) };
        if ret != 0 {
            return Err(io::Error::from_raw_os_error(ret));
        }
        Ok(())
    }

    /// Close `fd` in the child.
    fn add_close(&mut self, fd: c_int) -> io::Result<()> {
        // SAFETY: self.inner was successfully initialized.
        let ret = unsafe { libc::posix_spawn_file_actions_addclose(&mut self.inner, fd) };
        if ret != 0 {
            return Err(io::Error::from_raw_os_error(ret));
        }
        Ok(())
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: self.inner was successfully initialized and is being destroyed once.
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut self.inner);
        }
    }
}

/// Configuration for a disclaimed spawn.
pub struct DisclaimedSpawn<'a> {
    /// Executable path.
    pub exe: &'a Path,
    /// Arguments (argv), including argv[0].
    pub argv: &'a [&'a str],
    /// Additional environment variables to set (merged with current env).
    pub extra_env: &'a [(&'a str, &'a str)],
    /// If set, the child's stderr is redirected to this file.
    /// stdin and stdout are always redirected to /dev/null.
    pub stderr_file: Option<std::fs::File>,
}

/// Result of a successful disclaimed spawn.
pub struct SpawnResult {
    /// PID of the child process.
    pub pid: u32,
    /// Whether the disclaim API was available and used.
    pub disclaimed: bool,
}

/// Spawn a child process in its own process group, with stdio detached,
/// and with TCC responsibility disclaimed (if the API is available).
///
/// If `responsibility_spawnattrs_setdisclaim` is not available at runtime,
/// the spawn proceeds without it and `SpawnResult::disclaimed` is `false`.
pub fn spawn(config: DisclaimedSpawn<'_>) -> io::Result<SpawnResult> {
    let dev_null = c"/dev/null";

    // Build spawn attributes: own process group + disclaim.
    let mut attrs = SpawnAttrs::new()?;
    attrs.set_flags(libc::POSIX_SPAWN_SETPGROUP as c_short)?;
    attrs.set_pgroup(0)?;

    let disclaimed = if let Some(disclaim_fn) = resolve_disclaim_fn() {
        // SAFETY: attrs was successfully initialized, disclaim_fn was resolved
        // from a known symbol with a known signature.
        let ret = unsafe { disclaim_fn(attrs.as_mut_ptr(), 1) };
        if ret != 0 {
            // Non-fatal: fall back to undisclaimed spawn.
            false
        } else {
            true
        }
    } else {
        false
    };

    // Build file actions: stdin/stdout → /dev/null, stderr → file or /dev/null.
    let mut file_actions = FileActions::new()?;
    file_actions.add_open(libc::STDIN_FILENO, dev_null, libc::O_RDONLY, 0)?;
    file_actions.add_open(libc::STDOUT_FILENO, dev_null, libc::O_WRONLY, 0)?;

    if let Some(ref f) = config.stderr_file {
        let raw_fd = f.as_raw_fd();
        file_actions.add_dup2(raw_fd, libc::STDERR_FILENO)?;
        // Close the original fd in the child so it doesn't leak.
        file_actions.add_close(raw_fd)?;
    } else {
        file_actions.add_open(libc::STDERR_FILENO, dev_null, libc::O_WRONLY, 0)?;
    }

    // Build argv as null-terminated CString array.
    let argv_cstrings: Vec<CString> = config
        .argv
        .iter()
        .map(|s| CString::new(*s))
        .collect::<Result<_, _>>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut argv_ptrs: Vec<*mut c_char> = argv_cstrings
        .iter()
        .map(|s| s.as_ptr().cast_mut())
        .collect();
    argv_ptrs.push(std::ptr::null_mut());

    // Build envp: current environment + extra vars.
    let mut env_cstrings: Vec<CString> = std::env::vars()
        .map(|(k, v)| CString::new(format!("{k}={v}")))
        .collect::<Result<_, _>>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    for &(k, v) in config.extra_env {
        env_cstrings.push(
            CString::new(format!("{k}={v}"))
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?,
        );
    }
    let mut envp_ptrs: Vec<*mut c_char> =
        env_cstrings.iter().map(|s| s.as_ptr().cast_mut()).collect();
    envp_ptrs.push(std::ptr::null_mut());

    // Build exe path as CString.
    let exe_cstring = CString::new(config.exe.as_os_str().as_encoded_bytes().to_vec())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    // Spawn.
    let mut pid: pid_t = 0;
    // SAFETY: all pointers (exe, file_actions, attrs, argv, envp) are valid
    // and null-terminated. The CString/Vec owners are alive for this call.
    let ret = unsafe {
        libc::posix_spawn(
            &mut pid,
            exe_cstring.as_ptr(),
            file_actions.as_ptr(),
            attrs.as_ptr(),
            argv_ptrs.as_ptr(),
            envp_ptrs.as_ptr(),
        )
    };

    if ret != 0 {
        return Err(io::Error::from_raw_os_error(ret));
    }

    Ok(SpawnResult {
        pid: pid as u32,
        disclaimed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The disclaim symbol should be resolvable on macOS 13+.
    #[test]
    fn disclaim_symbol_available() {
        assert!(
            resolve_disclaim_fn().is_some(),
            "responsibility_spawnattrs_setdisclaim not found via dlsym"
        );
    }

    /// Spawn /usr/bin/true with disclaim and verify it exits cleanly.
    #[test]
    fn spawn_true_disclaimed() {
        let result = spawn(DisclaimedSpawn {
            exe: Path::new("/usr/bin/true"),
            argv: &["true"],
            extra_env: &[],
            stderr_file: None,
        })
        .expect("spawn failed");

        assert!(result.disclaimed);
        assert!(result.pid > 0);

        // Reap the child.
        let mut status: c_int = 0;
        // SAFETY: pid is a valid child PID we just spawned.
        let waited = unsafe { libc::waitpid(result.pid as pid_t, &mut status, 0) };
        assert_eq!(waited, result.pid as pid_t);
        assert!(libc::WIFEXITED(status));
        assert_eq!(libc::WEXITSTATUS(status), 0);
    }

    /// Spawn with extra environment variables propagated.
    #[test]
    fn spawn_with_extra_env() {
        let result = spawn(DisclaimedSpawn {
            exe: Path::new("/usr/bin/env"),
            argv: &["env"],
            extra_env: &[("DISCLAIM_TEST_VAR", "hello")],
            stderr_file: None,
        })
        .expect("spawn failed");

        assert!(result.disclaimed);

        let mut status: c_int = 0;
        // SAFETY: pid is a valid child PID we just spawned.
        unsafe {
            libc::waitpid(result.pid as pid_t, &mut status, 0);
        }
        assert!(libc::WIFEXITED(status));
    }
}
