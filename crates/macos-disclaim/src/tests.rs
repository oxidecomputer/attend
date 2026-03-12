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
