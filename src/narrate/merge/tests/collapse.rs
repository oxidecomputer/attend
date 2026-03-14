use super::super::*;

/// Convert seconds to a UTC timestamp (for test brevity).
fn ts(secs: f64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds((secs * 1000.0) as i64)
}

// ── net_change_diffs ────────────────────────────────────────────────────────

/// Multiple FileDiff events to the same path collapse: the first `old` and
/// last `new` survive, with the latest timestamp.
#[test]
fn net_change_diffs_collapses_same_path() {
    let diffs = vec![
        (ts(1.0), "a.rs".into(), "v0".to_string(), "v1".to_string()),
        (ts(2.0), "a.rs".into(), "v1".to_string(), "v2".to_string()),
        (ts(3.0), "a.rs".into(), "v2".to_string(), "v3".to_string()),
    ];
    let result = net_change_diffs(diffs);
    assert_eq!(result.len(), 1, "same-path diffs collapse to one entry");
    let (ts_out, path, old, new) = &result[0];
    assert_eq!(path, "a.rs");
    assert_eq!(old, "v0", "old is from the earliest diff");
    assert_eq!(new, "v3", "new is from the latest diff");
    assert_eq!(*ts_out, ts(3.0), "timestamp is from the latest diff");
}

/// Diffs to paths A, B, A produce output in insertion order: A then B.
/// The second A updates the existing A entry rather than appending.
#[test]
fn net_change_diffs_preserves_order() {
    let diffs = vec![
        (
            ts(1.0),
            "a.rs".into(),
            "a_old".to_string(),
            "a_mid".to_string(),
        ),
        (
            ts(2.0),
            "b.rs".into(),
            "b_old".to_string(),
            "b_new".to_string(),
        ),
        (
            ts(3.0),
            "a.rs".into(),
            "a_mid".to_string(),
            "a_new".to_string(),
        ),
    ];
    let result = net_change_diffs(diffs);
    assert_eq!(result.len(), 2, "two unique paths");
    assert_eq!(result[0].1, "a.rs", "a.rs first (insertion order)");
    assert_eq!(result[1].1, "b.rs", "b.rs second (insertion order)");
    assert_eq!(result[0].2, "a_old", "a.rs old from first diff");
    assert_eq!(result[0].3, "a_new", "a.rs new from last diff");
}

// ── collapse_ext_selections ─────────────────────────────────────────────────

/// Progressive ExternalSelection events from the same app+window merge:
/// when a later selection contains an earlier one's text, the earlier is
/// replaced by the later (forward-merge).
#[test]
fn collapse_ext_forward_merge() {
    let events = vec![
        Event::ExternalSelection {
            timestamp: ts(1.0),
            last_seen: ts(1.0),
            app: "Safari".to_string(),
            window_title: "Docs".to_string(),
            text: "hello".to_string(),
        },
        Event::ExternalSelection {
            timestamp: ts(2.0),
            last_seen: ts(2.0),
            app: "Safari".to_string(),
            window_title: "Docs".to_string(),
            text: "hello world".to_string(),
        },
    ];
    let result = collapse_ext_selections(events);
    assert_eq!(result.len(), 1, "progressive selection forward-merged");
    if let Event::ExternalSelection {
        text, timestamp, ..
    } = &result[0]
    {
        assert_eq!(text, "hello world", "wider selection survives");
        assert_eq!(*timestamp, ts(2.0), "timestamp from the wider event");
    } else {
        panic!("expected ExternalSelection");
    }
}

/// When a postexec ShellCommand triggers `retain` to remove the preexec entry,
/// the HashMap indices for ExternalSelection merge targets are adjusted so that
/// subsequent forward-merges still reference the correct elements.
#[test]
fn collapse_ext_survives_shell_command_retain() {
    let events = vec![
        Event::ExternalSelection {
            timestamp: ts(1.0),
            last_seen: ts(1.0),
            app: "Safari".to_string(),
            window_title: "Docs".to_string(),
            text: "hello".to_string(),
        },
        Event::ShellCommand {
            timestamp: ts(2.0),
            shell: ShellKind::Fish,
            command: "ls".to_string(),
            cwd: "/tmp".into(),
            exit_status: None,
            duration_secs: None,
        },
        Event::ShellCommand {
            timestamp: ts(3.0),
            shell: ShellKind::Fish,
            command: "ls".to_string(),
            cwd: "/tmp".into(),
            exit_status: Some(0),
            duration_secs: Some(0.1),
        },
        Event::ExternalSelection {
            timestamp: ts(4.0),
            last_seen: ts(4.0),
            app: "Safari".to_string(),
            window_title: "Docs".to_string(),
            text: "hello world".to_string(),
        },
    ];
    let result = collapse_ext_selections(events);
    // The preexec ShellCommand is removed and replaced by the postexec.
    // The two Safari ExternalSelections should merge (hello -> hello world).
    assert_eq!(result.len(), 2, "merged ExtSel + postexec ShellCommand");
    // First should be the merged ExternalSelection.
    if let Event::ExternalSelection { text, .. } = &result[0] {
        assert_eq!(
            text, "hello world",
            "progressive selection forward-merged after retain"
        );
    } else {
        panic!(
            "expected ExternalSelection at index 0, got: {:?}",
            result[0]
        );
    }
    // Second should be the postexec ShellCommand.
    if let Event::ShellCommand {
        exit_status, cwd, ..
    } = &result[1]
    {
        assert_eq!(*exit_status, Some(0), "postexec survives");
        assert_eq!(cwd, "/tmp", "preexec cwd preserved on merged command");
    } else {
        panic!("expected ShellCommand at index 1, got: {:?}", result[1]);
    }
}

/// ExternalSelection events from different apps do not merge, even if one
/// text is a substring of the other.
#[test]
fn collapse_ext_different_sources_no_merge() {
    let events = vec![
        Event::ExternalSelection {
            timestamp: ts(1.0),
            last_seen: ts(1.0),
            app: "Safari".to_string(),
            window_title: "Docs".to_string(),
            text: "hello".to_string(),
        },
        Event::ExternalSelection {
            timestamp: ts(2.0),
            last_seen: ts(2.0),
            app: "Firefox".to_string(),
            window_title: "Docs".to_string(),
            text: "hello world".to_string(),
        },
    ];
    let result = collapse_ext_selections(events);
    assert_eq!(result.len(), 2, "different apps do not merge");
}
