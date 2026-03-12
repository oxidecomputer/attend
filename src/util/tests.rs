use super::*;

/// Normal replace: staging content replaces existing directory.
#[test]
fn atomic_replace_dir_normal() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("target");

    // Initial content.
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("old.txt"), "old").unwrap();

    // Replace.
    atomic_replace_dir(&dir, &[("new.txt", "new")]).unwrap();

    assert!(dir.join("new.txt").exists());
    assert!(!dir.join("old.txt").exists());
    assert_eq!(fs::read_to_string(dir.join("new.txt")).unwrap(), "new");

    // No leftover staging or .old dirs.
    assert!(!dir.with_extension("staging").exists());
    assert!(!dir.with_extension("old").exists());
}

/// First call: directory doesn't exist yet.
#[test]
fn atomic_replace_dir_first_call() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("fresh");

    atomic_replace_dir(&dir, &[("a.txt", "hello")]).unwrap();

    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "hello");
}

/// Recovery: .old exists without dir (crash between step 1 and 2).
#[test]
fn atomic_replace_dir_recovers_old_without_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("target");
    let old = dir.with_extension("old");

    // Simulate crash state: .old exists, dir does not.
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("preserved.txt"), "saved").unwrap();

    // Recovery should restore .old -> dir, then replace with new content.
    atomic_replace_dir(&dir, &[("new.txt", "fresh")]).unwrap();

    assert_eq!(fs::read_to_string(dir.join("new.txt")).unwrap(), "fresh");
    assert!(!old.exists());
}

/// Recovery: both .old and dir exist (crash after step 2, before cleanup).
#[test]
fn atomic_replace_dir_cleans_stale_old() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("target");
    let old = dir.with_extension("old");

    // Simulate: both exist (step 3 cleanup didn't run).
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("current.txt"), "current").unwrap();
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("stale.txt"), "stale").unwrap();

    atomic_replace_dir(&dir, &[("replaced.txt", "done")]).unwrap();

    assert_eq!(
        fs::read_to_string(dir.join("replaced.txt")).unwrap(),
        "done"
    );
    assert!(!old.exists());
}

/// Recovery: leftover staging directory from prior crash is cleaned up.
#[test]
fn atomic_replace_dir_cleans_stale_staging() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("target");
    let staging = dir.with_extension("staging");

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("existing.txt"), "existing").unwrap();

    // Leftover staging from a prior incomplete write.
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("partial.txt"), "partial").unwrap();

    atomic_replace_dir(&dir, &[("final.txt", "complete")]).unwrap();

    assert_eq!(
        fs::read_to_string(dir.join("final.txt")).unwrap(),
        "complete"
    );
    assert!(!staging.exists());
}
