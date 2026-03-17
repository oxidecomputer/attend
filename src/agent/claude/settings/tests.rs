use camino::Utf8PathBuf;

use super::*;

/// All hook event names, derived from the shared hook definitions.
fn all_hook_keys() -> Vec<String> {
    hook_defs().into_iter().map(|d| d.event).collect()
}

/// Helper: read the settings JSON from a project tempdir.
fn read_settings(project: &std::path::Path) -> serde_json::Value {
    let path = project.join(".claude/settings.local.json");
    let content = fs::read_to_string(path).unwrap();
    serde_json::from_str(&content).unwrap()
}

/// Helper: convert a tempdir path to a Utf8PathBuf.
fn project_path(dir: &tempfile::TempDir) -> Utf8PathBuf {
    Utf8PathBuf::try_from(dir.path().to_path_buf()).unwrap()
}

/// Installing into a project with no existing settings creates hooks and permissions.
#[test]
fn install_fresh_project() {
    let dir = tempfile::tempdir().unwrap();
    install::install("attend", Some(project_path(&dir))).unwrap();

    let settings = read_settings(dir.path());
    let hooks = settings["hooks"].as_object().unwrap();

    // All hook keys from the shared defs should be present.
    for key in all_hook_keys() {
        assert!(hooks.contains_key(&key), "{key} should be present");

        let arr = hooks[&key].as_array().unwrap();
        assert_eq!(arr.len(), 1, "{key} should have exactly one entry");
        assert!(is_our_hook(&arr[0]), "{key} should be our hook");
    }

    // Permissions should include attend look and listen
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    assert!(allow.iter().any(|v| v.as_str().unwrap().contains("look")));
    assert!(allow.iter().any(|v| v.as_str().unwrap().contains("listen")));
}

/// Installing twice is idempotent: no duplicate entries.
#[test]
fn install_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let proj = project_path(&dir);
    install::install("attend", Some(proj.clone())).unwrap();
    install::install("attend", Some(proj)).unwrap();

    let settings = read_settings(dir.path());
    let hooks = settings["hooks"].as_object().unwrap();

    // Each hook array should still have exactly one entry
    for key in all_hook_keys() {
        let arr = hooks[&key].as_array().unwrap();
        assert_eq!(
            arr.len(),
            1,
            "{key} should have exactly one entry after re-install"
        );
    }

    // Permissions should not have duplicate entries
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    let attend_entries: Vec<_> = allow
        .iter()
        .filter(|v| {
            v.as_str()
                .is_some_and(|s| s.contains("attend look") || s.contains("attend listen"))
        })
        .collect();
    assert_eq!(
        attend_entries.len(),
        2,
        "should have exactly look + listen permissions"
    );
}

/// Install preserves existing non-attend hooks.
#[test]
fn install_preserves_other_hooks() {
    let dir = tempfile::tempdir().unwrap();
    let settings_dir = dir.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();

    // Write existing settings with a non-attend hook
    let existing = serde_json::json!({
        "hooks": {
            "SessionStart": [
                {
                    "matcher": "startup",
                    "hooks": [{"type": "command", "command": "echo hello"}]
                }
            ]
        }
    });
    fs::write(
        settings_dir.join("settings.local.json"),
        serde_json::to_string_pretty(&existing).unwrap(),
    )
    .unwrap();

    install::install("attend", Some(project_path(&dir))).unwrap();

    let settings = read_settings(dir.path());
    let session_start = settings["hooks"]["SessionStart"].as_array().unwrap();

    // Should have the original hook plus our new one
    assert_eq!(session_start.len(), 2);
    assert!(
        !is_our_hook(&session_start[0]),
        "first should be the original"
    );
    assert!(is_our_hook(&session_start[1]), "second should be ours");
}

/// Uninstall removes attend hooks but leaves others intact.
#[test]
fn uninstall_leaves_other_hooks() {
    let dir = tempfile::tempdir().unwrap();
    let proj = project_path(&dir);

    // Install attend hooks
    install::install("attend", Some(proj.clone())).unwrap();

    // Add a non-attend hook manually
    let mut settings = read_settings(dir.path());
    let ss_arr = settings["hooks"]["SessionStart"].as_array_mut().unwrap();
    ss_arr.push(serde_json::json!({
        "matcher": "startup",
        "hooks": [{"type": "command", "command": "echo other"}]
    }));
    let path = dir.path().join(".claude/settings.local.json");
    fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

    // Uninstall
    uninstall::uninstall(Some(proj)).unwrap();

    let settings = read_settings(dir.path());
    let hooks = settings["hooks"].as_object().unwrap();

    // SessionStart should have only the non-attend hook
    let ss_arr = hooks["SessionStart"].as_array().unwrap();
    assert_eq!(ss_arr.len(), 1);
    assert!(!is_our_hook(&ss_arr[0]));

    // Other hook arrays should be empty (only had attend entries)
    for key in all_hook_keys() {
        if key == "SessionStart" {
            continue;
        }
        let arr = hooks[&key].as_array().unwrap();
        assert!(arr.is_empty(), "{key} should be empty after uninstall");
    }

    // Attend permissions should be removed
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    assert!(
        !allow.iter().any(|v| v
            .as_str()
            .is_some_and(|s| s.contains("attend look") || s.contains("attend listen"))),
        "attend permissions should be removed"
    );
}

/// Uninstall with no settings file is a no-op (doesn't error).
#[test]
fn uninstall_no_settings_file() {
    let dir = tempfile::tempdir().unwrap();
    // Should succeed even with no settings file
    uninstall::uninstall(Some(project_path(&dir))).unwrap();
}

/// Uninstall with no attend hooks is a no-op.
#[test]
fn uninstall_no_attend_hooks() {
    let dir = tempfile::tempdir().unwrap();
    let settings_dir = dir.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();

    let existing = serde_json::json!({
        "hooks": {
            "SessionStart": [
                {"matcher": "startup", "hooks": [{"type": "command", "command": "echo hi"}]}
            ]
        }
    });
    fs::write(
        settings_dir.join("settings.local.json"),
        serde_json::to_string_pretty(&existing).unwrap(),
    )
    .unwrap();

    uninstall::uninstall(Some(project_path(&dir))).unwrap();

    // File should be unchanged (no attend hooks to remove)
    let settings = read_settings(dir.path());
    let ss_arr = settings["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(ss_arr.len(), 1);
}

/// Install then uninstall produces a clean (hook-free) settings file.
#[test]
fn install_uninstall_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let proj = project_path(&dir);

    install::install("attend", Some(proj.clone())).unwrap();
    uninstall::uninstall(Some(proj)).unwrap();

    let settings = read_settings(dir.path());
    let hooks = settings["hooks"].as_object().unwrap();

    // All hook arrays should be empty
    for key in all_hook_keys() {
        let arr = hooks[&key].as_array().unwrap();
        assert!(arr.is_empty(), "{key} should be empty after round-trip");
    }
}

/// is_our_hook correctly identifies attend hooks by marker.
#[test]
fn is_our_hook_marker() {
    let ours = serde_json::json!({HOOK_MARKER_KEY: HOOK_MARKER_VALUE, "hooks": []});
    let other = serde_json::json!({"hooks": [{"type": "command", "command": "echo"}]});
    let wrong_value = serde_json::json!({HOOK_MARKER_KEY: "other-tool", "hooks": []});

    assert!(is_our_hook(&ours));
    assert!(!is_our_hook(&other));
    assert!(!is_our_hook(&wrong_value));
}

/// Legacy hook entries (pre-marker) are identified by command pattern.
#[test]
fn is_our_hook_legacy_fallback() {
    // Legacy entry without marker but with attend command.
    let legacy = serde_json::json!({
        "hooks": [{"type": "command", "command": "attend hook session-start --agent claude"}]
    });
    assert!(is_our_hook(&legacy));

    // Bare "attend" command (edge case).
    let bare = serde_json::json!({
        "hooks": [{"type": "command", "command": "attend"}]
    });
    assert!(is_our_hook(&bare));

    // Dev install with absolute path.
    let dev = serde_json::json!({
        "hooks": [{"type": "command", "command": "/Users/me/src/attend/target/debug/attend hook session-start --agent claude"}]
    });
    assert!(is_our_hook(&dev));

    // Non-attend command should not match.
    let other = serde_json::json!({
        "hooks": [{"type": "command", "command": "some-other-tool hook"}]
    });
    assert!(!is_our_hook(&other));

    // Command containing "attend" as substring should not match.
    let substring = serde_json::json!({
        "hooks": [{"type": "command", "command": "unattend something"}]
    });
    assert!(!is_our_hook(&substring));
}

/// settings_path uses project-local path for project installs.
#[test]
fn settings_path_project() {
    let path = settings_path(Some(std::path::Path::new("/my/project"))).unwrap();
    assert_eq!(
        path,
        std::path::PathBuf::from("/my/project/.claude/settings.local.json")
    );
}

/// settings_path uses home directory for global installs.
#[test]
fn settings_path_global() {
    let path = settings_path(None).unwrap();
    assert!(path.ends_with(".claude/settings.json"));
}

/// Install preserves unrelated permissions that contain "attend" as a substring.
///
/// Projects named "attend" (or whose path contains it) may have permissions
/// with "attend" in the string. The install filter must not clobber them.
#[test]
fn install_preserves_unrelated_permissions_containing_attend() {
    let dir = tempfile::tempdir().unwrap();
    let settings_dir = dir.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();

    let existing = serde_json::json!({
        "permissions": {
            "allow": [
                "Bash(commit:*) /Users/oxide/src/attend",
                "Skill(commit-commands:commit)"
            ]
        }
    });
    fs::write(
        settings_dir.join("settings.local.json"),
        serde_json::to_string_pretty(&existing).unwrap(),
    )
    .unwrap();

    install::install("attend", Some(project_path(&dir))).unwrap();

    let settings = read_settings(dir.path());
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    let strings: Vec<&str> = allow.iter().filter_map(|v| v.as_str()).collect();

    assert!(
        strings.contains(&"Bash(commit:*) /Users/oxide/src/attend"),
        "unrelated permission with 'attend' substring should survive"
    );
    assert!(
        strings.contains(&"Skill(commit-commands:commit)"),
        "unrelated permission should survive"
    );
}

/// Install with malformed existing JSON returns an error.
#[test]
fn install_malformed_json_errors() {
    let dir = tempfile::tempdir().unwrap();
    let settings_dir = dir.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(settings_dir.join("settings.local.json"), "not json!!!").unwrap();

    let result = install::install("attend", Some(project_path(&dir)));
    assert!(result.is_err());
}

/// Install updates the bin_cmd in hook commands.
#[test]
fn install_uses_bin_cmd() {
    let dir = tempfile::tempdir().unwrap();
    install::install("/usr/local/bin/attend", Some(project_path(&dir))).unwrap();

    let settings = read_settings(dir.path());
    let ss_hooks = &settings["hooks"]["SessionStart"][0]["hooks"][0];
    let cmd = ss_hooks["command"].as_str().unwrap();
    assert!(cmd.starts_with("/usr/local/bin/attend"));
}

/// Helper: count attend hook entries across all hook keys.
fn count_attend_hooks(settings: &serde_json::Value) -> usize {
    let hooks = settings["hooks"].as_object().unwrap();
    all_hook_keys()
        .iter()
        .map(|key| {
            hooks
                .get(key.as_str())
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter(|e| is_our_hook(e)).count())
                .unwrap_or(0)
        })
        .sum()
}

/// Installing many times produces exactly one attend entry per hook key.
///
/// Reproduces the real-world bug where repeated installs accumulated
/// duplicate hook entries, causing hooks to fire N times per event.
#[test]
fn install_many_times_no_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let proj = project_path(&dir);

    for _ in 0..6 {
        install::install("attend", Some(proj.clone())).unwrap();
    }

    let settings = read_settings(dir.path());
    let hooks = settings["hooks"].as_object().unwrap();
    let keys = all_hook_keys();

    for key in &keys {
        let arr = hooks[key.as_str()].as_array().unwrap();
        let ours: Vec<_> = arr.iter().filter(|e| is_our_hook(e)).collect();
        assert_eq!(
            ours.len(),
            1,
            "{key} should have exactly one attend entry after 6 installs, got {}",
            ours.len()
        );
    }

    assert_eq!(
        count_attend_hooks(&settings),
        keys.len(),
        "total attend hooks should equal number of hook keys"
    );
}

/// Uninstalling twice is idempotent: second uninstall is a no-op.
#[test]
fn uninstall_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let proj = project_path(&dir);

    install::install("attend", Some(proj.clone())).unwrap();
    uninstall::uninstall(Some(proj.clone())).unwrap();

    // Capture state after first uninstall
    let after_first = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();

    // Second uninstall should not error and should not change the file
    uninstall::uninstall(Some(proj)).unwrap();
    let after_second = fs::read_to_string(dir.path().join(".claude/settings.local.json")).unwrap();

    assert_eq!(
        after_first, after_second,
        "second uninstall should be a no-op"
    );
}

/// Install after uninstall restores hooks cleanly (full round-trip cycle).
#[test]
fn install_uninstall_install_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let proj = project_path(&dir);

    install::install("attend", Some(proj.clone())).unwrap();
    let after_first_install = read_settings(dir.path());

    uninstall::uninstall(Some(proj.clone())).unwrap();
    install::install("attend", Some(proj)).unwrap();
    let after_reinstall = read_settings(dir.path());

    // Hook structure should be identical after reinstall
    assert_eq!(
        after_first_install["hooks"], after_reinstall["hooks"],
        "hooks should match after install-uninstall-install cycle"
    );
}

/// When the attend plugin is enabled, install writes only permissions
/// (no hooks, no skill files).
#[test]
fn install_plugin_mode_permissions_only() {
    let dir = tempfile::tempdir().unwrap();
    let settings_dir = dir.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();

    // Simulate the plugin being enabled.
    let existing = serde_json::json!({
        "enabledPlugins": { "attend@attend": true }
    });
    fs::write(
        settings_dir.join("settings.local.json"),
        serde_json::to_string_pretty(&existing).unwrap(),
    )
    .unwrap();

    install::install("attend", Some(project_path(&dir))).unwrap();

    let settings = read_settings(dir.path());

    // No hooks should be written.
    assert!(
        settings.get("hooks").is_none(),
        "plugin mode should not write hooks"
    );

    // Permissions should still be present.
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    assert!(allow.iter().any(|v| v.as_str().unwrap().contains("look")));
    assert!(allow
        .iter()
        .any(|v| v.as_str().unwrap().contains("listen")));

    // No skill files should be created.
    assert!(
        !dir.path().join(".claude/skills/attend").exists(),
        "plugin mode should not write skill files"
    );
    assert!(
        !dir.path().join(".claude/skills/unattend").exists(),
        "plugin mode should not write skill files"
    );
}

// ---------------------------------------------------------------------------
// Plugin parity tests
//
// The manual install (`attend install --agent claude`) and the plugin
// (`plugin/`) are generated from the same sources. These tests verify they
// stay structurally equivalent.
// ---------------------------------------------------------------------------

/// The hooks written by manual install match the plugin's hooks.json.
///
/// Uses `bin_cmd = "attend"` so the command strings are identical.
/// The only expected difference is the `_installed_by` marker that the
/// manual install adds for idempotent re-install/uninstall.
#[test]
fn manual_hooks_match_plugin() {
    let dir = tempfile::tempdir().unwrap();
    install::install("attend", Some(project_path(&dir))).unwrap();

    let settings = read_settings(dir.path());
    let installed = settings["hooks"].as_object().unwrap();

    let plugin_json: serde_json::Value =
        serde_json::from_str(include_str!("../../../../plugin/hooks/hooks.json")).unwrap();
    let plugin = plugin_json["hooks"].as_object().unwrap();

    // Same set of hook events.
    let installed_events: std::collections::BTreeSet<&str> =
        installed.keys().map(|s| s.as_str()).collect();
    let plugin_events: std::collections::BTreeSet<&str> =
        plugin.keys().map(|s| s.as_str()).collect();
    assert_eq!(
        installed_events, plugin_events,
        "manual and plugin should install the same hook events"
    );

    // For each event, the hook entry should be identical after stripping
    // the _installed_by marker.
    for event in &installed_events {
        let installed_arr = installed[*event].as_array().unwrap();
        let plugin_arr = plugin[*event].as_array().unwrap();

        assert_eq!(
            installed_arr.len(),
            plugin_arr.len(),
            "{event}: should have the same number of entries"
        );

        for (i, (inst, plug)) in installed_arr.iter().zip(plugin_arr.iter()).enumerate() {
            let mut normalized = inst.clone();
            normalized.as_object_mut().unwrap().remove(HOOK_MARKER_KEY);
            assert_eq!(
                normalized, *plug,
                "{event}[{i}]: manual install (minus marker) should equal plugin"
            );
        }
    }
}

/// The /attend SKILL.md from manual install matches the plugin's
/// /attend:start SKILL.md, modulo the parameterized names.
///
/// Substitutions that differ between the two paths:
///   - skill name: "attend" vs "start"
///   - stop skill: "/unattend" vs "/attend:stop"
///   - start skill (in protocol): "/attend" vs "/attend:start"
///
/// After normalizing these, the content should be identical.
#[test]
fn manual_start_skill_matches_plugin() {
    let dir = tempfile::tempdir().unwrap();
    install::install("attend", Some(project_path(&dir))).unwrap();

    let manual = fs::read_to_string(dir.path().join(".claude/skills/attend/SKILL.md")).unwrap();
    let plugin = include_str!("../../../../plugin/skills/start/SKILL.md");

    // Normalize manual install to plugin conventions.
    // Skill invocations are always backtick-delimited, so target those
    // precisely to avoid mangling "attend" in cache paths.
    let normalized = manual
        .replace("name: attend\n", "name: start\n")
        .replace("`/unattend`", "`/attend:stop`")
        .replace("`/attend`", "`/attend:start`");

    assert_eq!(
        normalized, plugin,
        "manual /attend SKILL.md should match plugin /attend:start SKILL.md \
         after normalizing parameterized names"
    );
}

/// The /unattend SKILL.md from manual install matches the plugin's
/// /attend:stop SKILL.md, modulo the skill name.
#[test]
fn manual_stop_skill_matches_plugin() {
    let dir = tempfile::tempdir().unwrap();
    install::install("attend", Some(project_path(&dir))).unwrap();

    let manual = fs::read_to_string(dir.path().join(".claude/skills/unattend/SKILL.md")).unwrap();
    let plugin = include_str!("../../../../plugin/skills/stop/SKILL.md");

    let normalized = manual.replace("name: unattend\n", "name: stop\n");
    assert_eq!(
        normalized, plugin,
        "manual /unattend SKILL.md should match plugin /attend:stop SKILL.md \
         after normalizing the skill name"
    );
}
