//! Handlers for the `install` and `uninstall` subcommands.

use camino::Utf8PathBuf;

/// Run the install subcommand.
pub(super) fn install(
    agent: Vec<String>,
    editor: Vec<String>,
    project: Option<Utf8PathBuf>,
    dev: bool,
) -> anyhow::Result<()> {
    if agent.is_empty() && editor.is_empty() {
        anyhow::bail!(
            "specify at least one --agent or --editor.\n  Available agents: {}\n  Available editors: {}",
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
        dev,
        project_paths,
    });
    Ok(())
}

/// Run the uninstall subcommand.
pub(super) fn uninstall(
    agent: Vec<String>,
    editor: Vec<String>,
    project: Option<Utf8PathBuf>,
) -> anyhow::Result<()> {
    let uninstall_all = agent.is_empty() && editor.is_empty();
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
    Ok(())
}
