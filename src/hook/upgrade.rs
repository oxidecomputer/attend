use crate::state;

/// Auto-upgrade hooks and editor integration when the running binary version
/// doesn't match the version that originally installed the hooks.
pub(super) fn auto_upgrade_hooks() {
    let Some(meta) = state::installed_meta() else {
        return;
    };
    let running = env!("CARGO_PKG_VERSION");
    if meta.version == running {
        return;
    }

    tracing::info!(
        installed = meta.version,
        running,
        "Version mismatch: reinstalling hooks"
    );

    let bin_cmd = match crate::agent::resolve_bin_cmd(meta.dev) {
        Ok(cmd) => cmd,
        Err(e) => {
            tracing::warn!("Cannot resolve bin command for auto-upgrade: {e}");
            return;
        }
    };

    for name in &meta.agents {
        if let Err(e) = crate::agent::install(name, None, meta.dev) {
            tracing::warn!(agent = name, "Auto-upgrade failed for agent: {e}");
        }
    }
    for name in &meta.editors {
        if let Some(ed) = crate::editor::editor_by_name(name)
            && let Err(e) = ed.install_narration(&bin_cmd)
        {
            tracing::warn!(editor = name, "Auto-upgrade failed for editor: {e}");
        }
    }

    state::save_install_meta(&state::InstallMeta {
        version: running.to_string(),
        agents: meta.agents,
        editors: meta.editors,
        dev: meta.dev,
        project_paths: meta.project_paths,
    });

    eprintln!("attend: hooks upgraded from {} to {running}", meta.version);
}
