//! Handlers for the `install` and `uninstall` subcommands.

use camino::Utf8PathBuf;

/// Run the install subcommand.
pub(super) fn install(
    agent: Vec<String>,
    editor: Vec<String>,
    browser: Vec<String>,
    project: Option<Utf8PathBuf>,
    dev: bool,
) -> anyhow::Result<()> {
    if agent.is_empty() && editor.is_empty() && browser.is_empty() {
        anyhow::bail!(
            "specify at least one --agent, --editor, or --browser.\n  \
             Available agents: {}\n  \
             Available editors: {}\n  \
             Available browsers: {}",
            crate::agent::AGENTS
                .iter()
                .map(|a| a.name())
                .collect::<Vec<_>>()
                .join(", "),
            crate::editor::EDITORS
                .iter()
                .map(|e| e.name())
                .collect::<Vec<_>>()
                .join(", "),
            crate::browser::BROWSERS
                .iter()
                .map(|b| b.name())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    let bin_cmd = crate::agent::resolve_bin_cmd(dev)?;
    for name in &agent {
        crate::agent::install(name, project.clone(), dev)?;
    }
    for name in &editor {
        let ed = crate::editor::editor_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("unknown editor: {name}"))?;
        ed.install_narration(&bin_cmd)?;
    }
    for name in &browser {
        let br = crate::browser::browser_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("unknown browser: {name}"))?;
        // The browser bridge binary must be the full path to `attend browser-bridge`.
        // The native messaging host manifest's "path" field points to the attend
        // binary; Firefox appends the subcommand via the manifest "type": "stdio".
        // Wait — native messaging launches the binary directly with no args.
        // The manifest "path" must point to a binary that speaks the native
        // messaging protocol on stdin/stdout. We need a wrapper or we need
        // the binary to detect it's being launched as a native messaging host.
        //
        // Solution: the manifest "path" points to `attend` and the manifest
        // doesn't support arguments. We'll detect the native messaging context
        // by checking if stdin is not a terminal (browser pipes) and use a
        // wrapper script, OR we use the `browser-bridge` subcommand with
        // a small shell wrapper.
        //
        // Actually, Firefox's native messaging protocol specifies that the
        // manifest "path" is the full path to the executable, and Firefox
        // passes the extension ID as the sole command-line argument. We need
        // the binary to handle this. The simplest approach: make `attend`
        // check if argv[1] matches an extension ID pattern and dispatch to
        // browser-bridge mode. But that's fragile.
        //
        // Better: use a wrapper script that calls `attend browser-bridge`.
        // The native_messaging crate doesn't handle this, so we write a
        // small shell script.
        //
        // Simplest: write a shell wrapper at the attend binary location.
        // E.g., ~/.cargo/bin/attend-browser-bridge that does:
        //   #!/bin/sh
        //   exec attend browser-bridge
        //
        // Actually, looking at this more carefully: Firefox native messaging
        // does NOT pass any arguments to the host binary in `sendNativeMessage`
        // mode (one-shot). It just launches the binary and pipes to stdin/stdout.
        // So we need a dedicated binary or wrapper.
        //
        // Let's write a small wrapper script next to the attend binary.
        let wrapper_path = install_browser_wrapper(&bin_cmd)?;
        br.install(&wrapper_path)?;
    }

    // Track project paths: preserve existing, append new (deduplicated).
    let mut project_paths = crate::state::installed_meta()
        .map(|m| m.project_paths)
        .unwrap_or_default();
    if let Some(ref p) = project
        && !project_paths.contains(p)
    {
        project_paths.push(p.clone());
    }

    crate::state::save_install_meta(&crate::state::InstallMeta {
        version: env!("CARGO_PKG_VERSION").to_string(),
        agents: agent,
        editors: editor,
        browsers: browser,
        dev,
        project_paths,
    });
    Ok(())
}

/// Create a wrapper script that invokes `attend browser-bridge`.
///
/// Firefox's native messaging protocol launches the binary directly with no
/// subcommand arguments, so we need a small wrapper that delegates to
/// `attend browser-bridge`. The wrapper is placed next to the attend binary.
fn install_browser_wrapper(bin_cmd: &str) -> anyhow::Result<String> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    let bin_path = Path::new(bin_cmd);
    let wrapper_name = "attend-browser-bridge";
    let wrapper_path = bin_path
        .parent()
        .map(|p| p.join(wrapper_name))
        .unwrap_or_else(|| wrapper_name.into());

    let script = format!("#!/bin/sh\nexec {bin_cmd} browser-bridge\n");
    fs::write(&wrapper_path, &script)?;
    fs::set_permissions(&wrapper_path, fs::Permissions::from_mode(0o755))?;

    Ok(wrapper_path.to_string_lossy().to_string())
}

/// Run the uninstall subcommand.
pub(super) fn uninstall(
    agent: Vec<String>,
    editor: Vec<String>,
    browser: Vec<String>,
    project: Option<Utf8PathBuf>,
) -> anyhow::Result<()> {
    let uninstall_all = agent.is_empty() && editor.is_empty() && browser.is_empty();
    let agents: Vec<String> = if uninstall_all {
        crate::agent::AGENTS
            .iter()
            .map(|a| a.name().to_string())
            .collect()
    } else {
        agent
    };
    let editors: Vec<String> = if uninstall_all {
        crate::editor::EDITORS
            .iter()
            .map(|e| e.name().to_string())
            .collect()
    } else {
        editor
    };
    let browsers: Vec<String> = if uninstall_all {
        crate::browser::BROWSERS
            .iter()
            .map(|b| b.name().to_string())
            .collect()
    } else {
        browser
    };

    // When no --project is given, also uninstall from all tracked project paths.
    if project.is_none()
        && let Some(meta) = crate::state::installed_meta()
    {
        for path in &meta.project_paths {
            for name in &agents {
                // Best-effort: project dir may have been removed.
                let _ = crate::agent::uninstall(name, Some(path.clone()));
            }
        }
        // Clear tracked project paths.
        crate::state::save_install_meta(&crate::state::InstallMeta {
            project_paths: Vec::new(),
            ..meta
        });
    }

    for name in &agents {
        crate::agent::uninstall(name, project.clone())?;
    }
    for name in &editors {
        let ed = crate::editor::editor_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("unknown editor: {name}"))?;
        ed.uninstall_narration()?;
    }
    for name in &browsers {
        let br = crate::browser::browser_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("unknown browser: {name}"))?;
        br.uninstall()?;
        // Also remove the wrapper script (best-effort).
        remove_browser_wrapper();
    }
    Ok(())
}

/// Remove the browser bridge wrapper script (best-effort).
fn remove_browser_wrapper() {
    if let Ok(path) = which::which("attend") {
        let wrapper = path.with_file_name("attend-browser-bridge");
        let _ = std::fs::remove_file(wrapper);
    }
}
