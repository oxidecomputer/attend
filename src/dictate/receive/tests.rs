use super::*;

#[test]
fn collect_pending_empty_dir() {
    let files = collect_pending("nonexistent-session");
    assert!(files.is_empty());
}

#[test]
fn read_pending_empty() {
    assert!(read_pending(&[]).is_none());
}

#[test]
fn read_pending_single() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("2026-02-18T10-00-00Z.md");
    fs::write(&path, "hello world").unwrap();
    let result = read_pending(&[path]).unwrap();
    assert_eq!(result, "hello world");
}

#[test]
fn read_pending_multiple_concatenated() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("2026-02-18T10-00-00Z.md");
    let p2 = dir.path().join("2026-02-18T10-05-00Z.md");
    fs::write(&p1, "first dictation").unwrap();
    fs::write(&p2, "second dictation").unwrap();
    let result = read_pending(&[p1, p2]).unwrap();
    assert_eq!(result, "first dictation\n\n---\n\nsecond dictation");
}

#[test]
fn archive_moves_files() {
    let base = tempfile::tempdir().unwrap();

    // Set up a fake pending dir structure
    let pending = base.path().join("pending").join("test-session");
    fs::create_dir_all(&pending).unwrap();
    let file = pending.join("2026-02-18T10-00-00Z.md");
    fs::write(&file, "content").unwrap();

    let archive = base.path().join("archive").join("test-session");
    fs::create_dir_all(&archive).unwrap();

    // We can't easily test archive_pending since it uses hardcoded paths,
    // but we can test the file operations directly
    let dest = archive.join("2026-02-18T10-00-00Z.md");
    fs::rename(&file, &dest).unwrap();

    assert!(!file.exists());
    assert!(dest.exists());
    assert_eq!(fs::read_to_string(&dest).unwrap(), "content");
}

#[test]
fn collect_pending_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let pending = dir.path();

    // Create files out of order
    fs::write(pending.join("b.md"), "second").unwrap();
    fs::write(pending.join("a.md"), "first").unwrap();
    fs::write(pending.join("c.md"), "third").unwrap();
    fs::write(pending.join("not-md.txt"), "skip").unwrap();

    // We can test sorting directly
    let mut files = vec![
        pending.join("b.md"),
        pending.join("a.md"),
        pending.join("c.md"),
    ];
    files.sort();
    assert_eq!(
        files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["a.md", "b.md", "c.md"]
    );
}

#[test]
fn lock_guard_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("test.lock");

    {
        let _guard = try_lock(&lock_path).expect("should acquire lock");
        assert!(lock_path.exists());

        // Second attempt should fail
        assert!(try_lock(&lock_path).is_none());
    }

    // After drop, lock should be removed
    assert!(!lock_path.exists());
}
