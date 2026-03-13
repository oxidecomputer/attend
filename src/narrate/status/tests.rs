use super::*;

/// `format_size` returns "B" suffix for values below 1 KB.
#[test]
fn format_size_bytes() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(1023), "1023 B");
}

/// `format_size` returns "KB" suffix with one decimal place for values in [1 KB, 1 MB).
#[test]
fn format_size_kilobytes() {
    assert_eq!(format_size(1024), "1.0 KB");
    assert_eq!(format_size(1536), "1.5 KB");
}

/// `format_size` returns "MB" suffix with one decimal place for values in [1 MB, 1 GB).
#[test]
fn format_size_megabytes() {
    assert_eq!(format_size(1024 * 1024), "1.0 MB");
    assert_eq!(format_size(10 * 1024 * 1024 + 512 * 1024), "10.5 MB");
}

/// `format_size` returns "GB" suffix with one decimal place for values >= 1 GB.
#[test]
fn format_size_gigabytes() {
    assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    assert_eq!(
        format_size(2 * 1024 * 1024 * 1024 + 512 * 1024 * 1024),
        "2.5 GB"
    );
}

/// Display output includes all sections in the expected order and format
/// when there are no config warnings.
#[test]
fn display_basic_status() {
    let info = StatusInfo {
        recording: RecordingState::Recording,
        engine: EngineInfo {
            display_name: "Parakeet TDT",
            model_cached: true,
        },
        idle_timeout: "5m (default)".to_string(),
        session: Some("abc123".to_string()),
        listener: ListenerState::Active,
        editors: vec![IntegrationHealth {
            name: "zed".to_string(),
            warnings: vec![],
        }],
        shells: vec![IntegrationHealth {
            name: "fish".to_string(),
            warnings: vec![],
        }],
        browsers: vec![IntegrationHealth {
            name: "chrome".to_string(),
            warnings: vec![],
        }],
        accessibility: AccessibilityState::Ok,
        clipboard_enabled: true,
        pending_count: 3,
        archive_size: 1024 * 1024 * 5,
        paths: StatusPaths {
            cache: Utf8PathBuf::from("/tmp/cache"),
            archive: Utf8PathBuf::from("/tmp/cache/narration/archive"),
            lock: Utf8PathBuf::from("/tmp/cache/daemon/lock"),
            config: Some(Utf8PathBuf::from("/tmp/config/attend/config.toml")),
        },
        config_warnings: vec![],
    };

    let output = info.to_string();

    // Verify each expected line is present.
    assert!(output.contains("Recording:      recording\n"));
    assert!(output.contains("Engine:         Parakeet TDT (model downloaded)\n"));
    assert!(output.contains("Idle timeout:   5m (default)\n"));
    assert!(output.contains("Session:        abc123\n"));
    assert!(output.contains("Listener:       active\n"));
    assert!(output.contains("Editors:        zed (ok)\n"));
    assert!(output.contains("Shells:         fish (ok)\n"));
    assert!(output.contains("Browsers:       chrome (ok)\n"));
    assert!(output.contains("Accessibility:  ok\n"));
    assert!(output.contains("Clipboard:      enabled\n"));
    assert!(output.contains("Pending:        3 narration(s)\n"));
    assert!(output.contains("Archive:        5.0 MB\n"));
    assert!(output.contains("Paths:\n"));
    assert!(output.contains("  Cache:      /tmp/cache\n"));
    assert!(output.contains("  Archive:    /tmp/cache/narration/archive\n"));
    assert!(output.contains("  Lock:       /tmp/cache/daemon/lock\n"));
    assert!(output.contains("  Config:     /tmp/config/attend/config.toml\n"));
    // No config warnings section.
    assert!(!output.contains("Config warnings:"));
}

/// Display output omits the session value as "none" when no session is active.
#[test]
fn display_no_session() {
    let info = minimal_status_info();
    let output = info.to_string();
    assert!(output.contains("Session:        none\n"));
}

/// Display output includes all recording state variants with correct text.
#[test]
fn display_recording_states() {
    let states = vec![
        (RecordingState::Stopped, "stopped"),
        (RecordingState::Idle, "idle (daemon resident)"),
        (RecordingState::Recording, "recording"),
        (
            RecordingState::StaleLock,
            "stale lock (daemon not running): run `attend narrate toggle` to clean up",
        ),
        (RecordingState::Unknown, "unknown (lock file unreadable)"),
    ];

    for (state, expected) in states {
        let mut info = minimal_status_info();
        info.recording = state;
        let output = info.to_string();
        assert!(
            output.contains(&format!("Recording:      {expected}\n")),
            "Expected recording text '{expected}' not found in output:\n{output}"
        );
    }
}

/// Display output includes all listener state variants with correct text.
#[test]
fn display_listener_states() {
    let states = vec![
        (ListenerState::Inactive, "inactive"),
        (ListenerState::Active, "active"),
        (ListenerState::StaleLock, "stale lock"),
        (ListenerState::Unknown, "unknown (lock file unreadable)"),
    ];

    for (state, expected) in states {
        let mut info = minimal_status_info();
        info.listener = state;
        let output = info.to_string();
        assert!(
            output.contains(&format!("Listener:       {expected}\n")),
            "Expected listener text '{expected}' not found in output:\n{output}"
        );
    }
}

/// Display output shows "not downloaded" when the model is not cached.
#[test]
fn display_model_not_cached() {
    let mut info = minimal_status_info();
    info.engine.model_cached = false;
    let output = info.to_string();
    assert!(output.contains("(model not downloaded)"));
}

/// Display output shows editor warnings inline when present.
#[test]
fn display_editor_warnings() {
    let mut info = minimal_status_info();
    info.editors = vec![IntegrationHealth {
        name: "zed".to_string(),
        warnings: vec!["task missing".to_string(), "keymap outdated".to_string()],
    }];
    let output = info.to_string();
    assert!(output.contains("Editors:        zed (task missing; keymap outdated)\n"));
}

/// Display output omits the Editors line when the list is empty.
#[test]
fn display_no_editors() {
    let mut info = minimal_status_info();
    info.editors = vec![];
    let output = info.to_string();
    assert!(!output.contains("Editors:"));
}

/// Display output omits the Shells line when the list is empty.
#[test]
fn display_no_shells() {
    let info = minimal_status_info();
    assert!(info.shells.is_empty());
    let output = info.to_string();
    assert!(!output.contains("Shells:"));
}

/// Display output omits the Browsers line when the list is empty.
#[test]
fn display_no_browsers() {
    let info = minimal_status_info();
    assert!(info.browsers.is_empty());
    let output = info.to_string();
    assert!(!output.contains("Browsers:"));
}

/// Display output includes config warnings when present.
#[test]
fn display_config_warnings() {
    let mut info = minimal_status_info();
    info.config_warnings =
        vec!["archive_retention: invalid value \"foo\" (using default 7d)".to_string()];
    let output = info.to_string();
    assert!(output.contains("\nConfig warnings:\n"));
    assert!(output.contains("  - archive_retention: invalid value \"foo\" (using default 7d)\n"));
}

/// Display output shows "disabled" for clipboard when disabled.
#[test]
fn display_clipboard_disabled() {
    let mut info = minimal_status_info();
    info.clipboard_enabled = false;
    let output = info.to_string();
    assert!(output.contains("Clipboard:      disabled\n"));
}

/// Display output omits the Config path line when no config home is available.
#[test]
fn display_no_config_path() {
    let mut info = minimal_status_info();
    info.paths.config = None;
    let output = info.to_string();
    assert!(!output.contains("Config:"));
}

/// Display output shows all accessibility state variants with correct text.
#[test]
fn display_accessibility_states() {
    let states = vec![
        (AccessibilityState::Ok, "ok"),
        (
            AccessibilityState::PermissionNotGranted,
            "permission not granted (System Settings > Privacy & Security > Accessibility)",
        ),
        (
            AccessibilityState::NotAvailable,
            "not available (no platform backend)",
        ),
    ];

    for (state, expected) in states {
        let mut info = minimal_status_info();
        info.accessibility = state;
        let output = info.to_string();
        assert!(
            output.contains(&format!("Accessibility:  {expected}\n")),
            "Expected accessibility text '{expected}' not found in output:\n{output}"
        );
    }
}

/// Build a minimal `StatusInfo` for tests that only need to override a few fields.
fn minimal_status_info() -> StatusInfo {
    StatusInfo {
        recording: RecordingState::Stopped,
        engine: EngineInfo {
            display_name: "Parakeet TDT",
            model_cached: true,
        },
        idle_timeout: "5m (default)".to_string(),
        session: None,
        listener: ListenerState::Inactive,
        editors: vec![],
        shells: vec![],
        browsers: vec![],
        accessibility: AccessibilityState::Ok,
        clipboard_enabled: true,
        pending_count: 0,
        archive_size: 0,
        paths: StatusPaths {
            cache: Utf8PathBuf::from("/tmp/cache"),
            archive: Utf8PathBuf::from("/tmp/cache/narration/archive"),
            lock: Utf8PathBuf::from("/tmp/cache/daemon/lock"),
            config: Some(Utf8PathBuf::from("/tmp/config/attend/config.toml")),
        },
        config_warnings: vec![],
    }
}
