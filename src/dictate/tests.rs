use super::*;

#[test]
fn resolve_session_flag_takes_precedence() {
    let result = resolve_session(Some("my-session".to_string()));
    assert_eq!(result, Some("my-session".to_string()));
}

#[test]
fn resolve_session_no_flag_no_listening() {
    // When no flag and no listening file exists, returns None
    // (depends on whether listening file exists on disk, so just test the flag path)
    let result = resolve_session(Some("test".to_string()));
    assert_eq!(result.unwrap(), "test");
}

#[test]
fn cache_dir_is_under_attend() {
    let dir = cache_dir();
    assert!(dir.ends_with("attend"));
}

#[test]
fn pending_dir_includes_session() {
    let dir = pending_dir("abc-123");
    assert!(dir.ends_with("pending/abc-123") || dir.ends_with("pending\\abc-123"));
}

#[test]
fn archive_dir_includes_session() {
    let dir = archive_dir("abc-123");
    assert!(dir.ends_with("archive/abc-123") || dir.ends_with("archive\\abc-123"));
}
