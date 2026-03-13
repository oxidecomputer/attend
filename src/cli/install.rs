//! Handlers for the `install` and `uninstall` subcommands.

use camino::Utf8PathBuf;

/// Arguments for the `install` subcommand.
#[derive(clap::Args)]
pub struct InstallArgs {
    /// Agent to install hooks for (repeatable).
    #[arg(long, short, value_parser = super::hook::agent_value_parser())]
    pub agent: Vec<String>,

    /// Editor to install narration keybindings for (repeatable).
    #[arg(long, short, value_parser = super::hook::editor_value_parser())]
    pub editor: Vec<String>,

    /// Browser to install native messaging for (repeatable).
    #[arg(long, short, value_parser = super::hook::browser_value_parser())]
    pub browser: Vec<String>,

    /// Shell to install hooks and completions for (repeatable).
    #[arg(long, short, value_parser = super::hook::shell_value_parser())]
    pub shell: Vec<String>,

    /// Install to a project-local settings file instead of global.
    #[arg(long, short)]
    pub project: Option<Utf8PathBuf>,

    /// Use absolute path to current binary instead of $PATH lookup.
    #[arg(long)]
    pub dev: bool,
}

impl InstallArgs {
    pub fn run(self) -> anyhow::Result<()> {
        let has_explicit = !self.agent.is_empty()
            || !self.editor.is_empty()
            || !self.browser.is_empty()
            || !self.shell.is_empty();

        if has_explicit {
            install_targeted(
                self.agent,
                self.editor,
                self.browser,
                self.shell,
                self.project,
                self.dev,
            )
        } else {
            install_auto(self.project, self.dev)
        }
    }
}

/// Arguments for the `uninstall` subcommand.
#[derive(clap::Args)]
pub struct UninstallArgs {
    /// Agent to uninstall hooks for (repeatable).
    #[arg(long, short, value_parser = super::hook::agent_value_parser())]
    pub agent: Vec<String>,

    /// Editor to uninstall narration keybindings for (repeatable).
    #[arg(long, value_parser = super::hook::editor_value_parser())]
    pub editor: Vec<String>,

    /// Browser to uninstall native messaging for (repeatable).
    #[arg(long, short, value_parser = super::hook::browser_value_parser())]
    pub browser: Vec<String>,

    /// Shell to uninstall hooks and completions for (repeatable).
    #[arg(long, short, value_parser = super::hook::shell_value_parser())]
    pub shell: Vec<String>,

    /// Remove from a project-local settings file instead of global.
    #[arg(long, short)]
    pub project: Option<Utf8PathBuf>,
}

impl UninstallArgs {
    pub fn run(self) -> anyhow::Result<()> {
        uninstall(
            self.agent,
            self.editor,
            self.browser,
            self.shell,
            self.project,
        )
    }
}

// ---------------------------------------------------------------------------
// Outcome tracking for auto-detect mode
// ---------------------------------------------------------------------------

/// Category label for display.
#[derive(Clone, Copy)]
enum Category {
    Agent,
    Editor,
    Browser,
    Shell,
}

impl Category {
    fn label(self) -> &'static str {
        match self {
            Category::Agent => "agent",
            Category::Editor => "editor",
            Category::Browser => "browser",
            Category::Shell => "shell",
        }
    }
}

/// Result of attempting to install a single integration.
enum Outcome {
    /// Successfully installed.
    Installed { category: Category, name: String },
    /// Skipped with a reason (not an error).
    Skipped {
        category: Category,
        name: String,
        reason: String,
    },
}

impl Outcome {
    fn is_installed(&self) -> bool {
        matches!(self, Outcome::Installed { .. })
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Installed { category, name } => {
                write!(f, "  + {}: {name}", category.label())
            }
            Outcome::Skipped {
                category,
                name,
                reason,
            } => {
                write!(f, "  - {}: {name} ({reason})", category.label())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Auto-detect install: try every known integration, report results
// ---------------------------------------------------------------------------

/// Install all known integrations, catching errors as skips.
///
/// Prints a summary and succeeds if at least one integration was installed.
fn install_auto(project: Option<Utf8PathBuf>, dev: bool) -> anyhow::Result<()> {
    let bin_cmd = crate::agent::resolve_bin_cmd(dev)?;
    let mut outcomes: Vec<Outcome> = Vec::new();

    // Agents
    for agent in crate::agent::AGENTS {
        let name = agent.name().to_string();
        match agent.install(&bin_cmd, project.clone()) {
            Ok(()) => outcomes.push(Outcome::Installed {
                category: Category::Agent,
                name,
            }),
            Err(e) => outcomes.push(Outcome::Skipped {
                category: Category::Agent,
                name,
                reason: concise_reason(&e),
            }),
        }
    }

    // Editors
    for editor in crate::editor::EDITORS {
        let name = editor.name().to_string();
        match editor.install_narration(&bin_cmd) {
            Ok(()) => outcomes.push(Outcome::Installed {
                category: Category::Editor,
                name,
            }),
            Err(e) => outcomes.push(Outcome::Skipped {
                category: Category::Editor,
                name,
                reason: concise_reason(&e),
            }),
        }
    }

    // Browsers
    let wrapper_path = install_browser_wrapper(&bin_cmd);
    for browser in crate::browser::BROWSERS {
        let name = browser.name().to_string();
        match &wrapper_path {
            Ok(wp) => match browser.install(wp) {
                Ok(()) => outcomes.push(Outcome::Installed {
                    category: Category::Browser,
                    name,
                }),
                Err(e) => outcomes.push(Outcome::Skipped {
                    category: Category::Browser,
                    name,
                    reason: concise_reason(&e),
                }),
            },
            Err(e) => outcomes.push(Outcome::Skipped {
                category: Category::Browser,
                name,
                reason: concise_reason(e),
            }),
        }
    }

    // Shells
    for shell in crate::shell::SHELLS {
        let name = shell.name().to_string();
        match shell.install_hooks(&bin_cmd) {
            Ok(()) => {
                // Hooks installed; also try completions (best-effort).
                if let Err(e) = shell.install_completions(&bin_cmd) {
                    tracing::warn!("shell {name}: completions failed: {e}");
                }
                outcomes.push(Outcome::Installed {
                    category: Category::Shell,
                    name,
                })
            }
            Err(e) => outcomes.push(Outcome::Skipped {
                category: Category::Shell,
                name,
                reason: concise_reason(&e),
            }),
        }
    }

    // Print summary
    let installed_count = outcomes.iter().filter(|o| o.is_installed()).count();
    println!();
    for outcome in &outcomes {
        println!("{outcome}");
    }
    println!();

    if installed_count == 0 {
        anyhow::bail!("no integrations were installed");
    }

    println!(
        "{installed_count} integration{} installed.",
        if installed_count == 1 { "" } else { "s" }
    );

    // Save metadata for successfully installed integrations only.
    let mut meta = crate::state::installed_meta().unwrap_or_default();
    meta.version = env!("CARGO_PKG_VERSION").to_string();
    meta.dev = dev;
    for outcome in &outcomes {
        if let Outcome::Installed { category, name } = outcome {
            let list = match category {
                Category::Agent => &mut meta.agents,
                Category::Editor => &mut meta.editors,
                Category::Browser => &mut meta.browsers,
                Category::Shell => &mut meta.shells,
            };
            if !list.contains(name) {
                list.push(name.clone());
            }
        }
    }
    if let Some(ref p) = project
        && !meta.project_paths.contains(p)
    {
        meta.project_paths.push(p.clone());
    }
    crate::state::save_install_meta(&meta);

    Ok(())
}

/// Extract a concise one-line reason from an error chain.
fn concise_reason(err: &anyhow::Error) -> String {
    // Use the root cause (innermost error) for brevity.
    format!("{}", err.root_cause())
}

// ---------------------------------------------------------------------------
// Targeted install: explicit flags, errors are fatal
// ---------------------------------------------------------------------------

/// Install specific integrations named by the user. Errors are fatal.
fn install_targeted(
    agent: Vec<String>,
    editor: Vec<String>,
    browser: Vec<String>,
    shell: Vec<String>,
    project: Option<Utf8PathBuf>,
    dev: bool,
) -> anyhow::Result<()> {
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
        let wrapper_path = install_browser_wrapper(&bin_cmd)?;
        br.install(&wrapper_path)?;
    }
    for name in &shell {
        let sh = crate::shell::shell_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("unknown shell: {name}"))?;
        sh.install_hooks(&bin_cmd)?;
        sh.install_completions(&bin_cmd)?;
    }

    // Merge with existing metadata so partial reinstalls don't clobber
    // previously installed integrations.
    let mut meta = crate::state::installed_meta().unwrap_or_default();
    meta.version = env!("CARGO_PKG_VERSION").to_string();
    meta.dev = dev;
    merge_unique(&mut meta.agents, agent);
    merge_unique(&mut meta.editors, editor);
    merge_unique(&mut meta.browsers, browser);
    merge_unique(&mut meta.shells, shell);
    if let Some(ref p) = project
        && !meta.project_paths.contains(p)
    {
        meta.project_paths.push(p.clone());
    }

    crate::state::save_install_meta(&meta);
    Ok(())
}

/// Merge `new` items into `existing`, skipping duplicates.
fn merge_unique(existing: &mut Vec<String>, new: Vec<String>) {
    for item in new {
        if !existing.contains(&item) {
            existing.push(item);
        }
    }
}

/// Return `explicit` if non-empty, otherwise collect all names from
/// the registry when `use_all` is set.
fn resolve_or_all(
    explicit: Vec<String>,
    all_names: impl Iterator<Item = &'static str>,
    use_all: bool,
) -> Vec<String> {
    if use_all {
        all_names.map(str::to_string).collect()
    } else {
        explicit
    }
}

/// Create a wrapper script that invokes `attend browser-bridge`.
///
/// Firefox's native messaging protocol launches the binary directly with no
/// subcommand arguments, so we need a small wrapper that delegates to
/// `attend browser-bridge`. The wrapper is placed next to the attend binary.
fn install_browser_wrapper(bin_cmd: &str) -> anyhow::Result<String> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    // Native messaging manifests require absolute paths. Resolve if needed.
    let abs_bin: PathBuf = if std::path::Path::new(bin_cmd).is_absolute() {
        bin_cmd.into()
    } else {
        which::which(bin_cmd).map_err(|e| anyhow::anyhow!("cannot find {bin_cmd} on PATH: {e}"))?
    };

    let wrapper_name = "attend-browser-bridge";
    let wrapper_path = abs_bin
        .parent()
        .map(|p| p.join(wrapper_name))
        .unwrap_or_else(|| wrapper_name.into());

    let script = format!("#!/bin/sh\nexec {} browser-bridge\n", abs_bin.display());
    fs::write(&wrapper_path, &script)?;
    fs::set_permissions(&wrapper_path, fs::Permissions::from_mode(0o755))?;

    Ok(wrapper_path.to_string_lossy().to_string())
}

/// Run the uninstall subcommand.
fn uninstall(
    agent: Vec<String>,
    editor: Vec<String>,
    browser: Vec<String>,
    shell: Vec<String>,
    project: Option<Utf8PathBuf>,
) -> anyhow::Result<()> {
    let uninstall_all =
        agent.is_empty() && editor.is_empty() && browser.is_empty() && shell.is_empty();
    let agents = resolve_or_all(
        agent,
        crate::agent::AGENTS.iter().map(|a| a.name()),
        uninstall_all,
    );
    let editors = resolve_or_all(
        editor,
        crate::editor::EDITORS.iter().map(|e| e.name()),
        uninstall_all,
    );
    let browsers = resolve_or_all(
        browser,
        crate::browser::BROWSERS.iter().map(|b| b.name()),
        uninstall_all,
    );
    let shells = resolve_or_all(
        shell,
        crate::shell::SHELLS.iter().map(|s| s.name()),
        uninstall_all,
    );

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
    for name in &shells {
        let sh = crate::shell::shell_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("unknown shell: {name}"))?;
        sh.uninstall_hooks()?;
        sh.uninstall_completions()?;
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

#[cfg(test)]
mod tests;
