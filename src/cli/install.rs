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
// Auto-detect install: detect available integrations, prompt, install
// ---------------------------------------------------------------------------

/// Check whether a known integration appears to be present on this system.
fn is_detected(category: Category, name: &str) -> bool {
    match (category, name) {
        (Category::Agent, "claude") => {
            which::which("claude").is_ok()
                || dirs::home_dir().is_some_and(|h| h.join(".claude").is_dir())
        }
        (Category::Editor, "zed") => which::which("zed").is_ok() || has_macos_app("Zed"),
        (Category::Browser, "firefox") => {
            which::which("firefox").is_ok() || has_macos_app("Firefox")
        }
        (Category::Browser, "chrome") => {
            which::which("google-chrome").is_ok()
                || which::which("google-chrome-stable").is_ok()
                || which::which("chromium-browser").is_ok()
                || which::which("chromium").is_ok()
                || has_macos_app("Google Chrome")
        }
        (Category::Shell, shell) => which::which(shell).is_ok(),
        _ => false,
    }
}

/// Check for a macOS `/Applications/*.app` bundle. Always returns `false`
/// on other platforms.
fn has_macos_app(_name: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        std::path::Path::new(&format!("/Applications/{_name}.app")).exists()
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Brief description of what each integration does, shown during prompting.
fn integration_description(category: Category, name: &str) -> &'static str {
    match (category, name) {
        (Category::Agent, "claude") => "Claude Code: allows using /attend in Claude",
        (Category::Editor, "zed") => "Zed keybindings: adds hotkeys for attend within Zed",
        (Category::Browser, "firefox") => {
            "Firefox extension: captures text you select on web pages while narrating"
        }
        (Category::Browser, "chrome") => {
            "Chrome extension: captures text you select on web pages while narrating"
        }
        (Category::Shell, "fish") => "Fish hooks: captures commands you run while narrating",
        (Category::Shell, "zsh") => "Zsh hooks: captures commands you run while narrating",
        _ => "",
    }
}

/// Read a single keypress: y/Y/Enter → `Some(true)`, n/N → `Some(false)`,
/// Ctrl-C → `None` (abort).
fn read_yn() -> Option<bool> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

    loop {
        let Ok(Event::Key(KeyEvent {
            code, modifiers, ..
        })) = event::read()
        else {
            continue;
        };
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            return None;
        }
        return match code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(false),
            _ => continue,
        };
    }
}

/// Prompt for a y/n answer with a single keypress (no Enter required).
/// Returns `Some(true)` for yes, `Some(false)` for no, `None` on Ctrl-C.
fn prompt_yn(label: &str) -> Option<bool> {
    use crossterm::terminal;
    use std::io::Write;

    print!("{label} [Y/n] ");
    std::io::stdout().flush().ok();

    // If we can't enter raw mode (e.g., not a real TTY), fall back to
    // line-based input.
    if terminal::enable_raw_mode().is_err() {
        return Some(prompt_yn_line());
    }

    let answer = read_yn();
    terminal::disable_raw_mode().ok();

    match answer {
        Some(true) => println!("y"),
        Some(false) => println!("n\n"),
        None => println!(),
    }

    answer
}

/// Line-based y/n fallback when raw mode is unavailable.
fn prompt_yn_line() -> bool {
    use std::io::BufRead;

    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return true;
    }
    let answer = line.trim();
    answer.is_empty() || answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
}

/// Route to interactive or non-interactive install.
fn install_auto(project: Option<Utf8PathBuf>, dev: bool) -> anyhow::Result<()> {
    use std::io::IsTerminal;

    if std::io::stdin().is_terminal() {
        install_interactive(project, dev)
    } else {
        install_noninteractive(project, dev)
    }
}

/// Interactive install: detect, prompt per integration, install immediately.
fn install_interactive(project: Option<Utf8PathBuf>, dev: bool) -> anyhow::Result<()> {
    let bin_cmd = crate::agent::resolve_bin_cmd(dev)?;
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut any_detected = false;
    let mut browser_wrapper: Option<anyhow::Result<String>> = None;

    println!(
        "These integrations allow attend to interleave what you do with what\n\
        you say, and deliver that combined narration to your coding agent:\n"
    );

    // Agents
    for agent in crate::agent::AGENTS {
        if !is_detected(Category::Agent, agent.name()) {
            continue;
        }
        any_detected = true;
        let desc = integration_description(Category::Agent, agent.name());
        match prompt_yn(desc) {
            Some(true) => {}
            Some(false) => continue,
            None => return Ok(()),
        }
        let name = agent.name().to_string();
        match agent.install(&bin_cmd, project.clone()) {
            Ok(()) => {
                outcomes.push(Outcome::Installed {
                    category: Category::Agent,
                    name,
                });
                println!();
            }
            Err(e) => {
                let reason = concise_reason(&e);
                println!("  Failed: {reason}");
                outcomes.push(Outcome::Skipped {
                    category: Category::Agent,
                    name,
                    reason,
                });
            }
        }
    }

    // Editors
    for editor in crate::editor::EDITORS {
        if !is_detected(Category::Editor, editor.name()) {
            continue;
        }
        any_detected = true;
        let desc = integration_description(Category::Editor, editor.name());
        match prompt_yn(desc) {
            Some(true) => {}
            Some(false) => continue,
            None => return Ok(()),
        }
        let name = editor.name().to_string();
        match editor.install_narration(&bin_cmd) {
            Ok(()) => {
                outcomes.push(Outcome::Installed {
                    category: Category::Editor,
                    name,
                });
                println!();
            }
            Err(e) => {
                let reason = concise_reason(&e);
                println!("  Failed: {reason}\n");
                outcomes.push(Outcome::Skipped {
                    category: Category::Editor,
                    name,
                    reason,
                });
            }
        }
    }

    // Browsers
    for browser in crate::browser::BROWSERS {
        if !is_detected(Category::Browser, browser.name()) {
            continue;
        }
        any_detected = true;
        let desc = integration_description(Category::Browser, browser.name());
        match prompt_yn(desc) {
            Some(true) => {}
            Some(false) => continue,
            None => return Ok(()),
        }
        // Lazily create the browser wrapper on first confirmed browser.
        let wp = browser_wrapper.get_or_insert_with(|| install_browser_wrapper(&bin_cmd));
        let name = browser.name().to_string();
        match wp {
            Ok(wp) => match browser.install(wp) {
                Ok(()) => {
                    outcomes.push(Outcome::Installed {
                        category: Category::Browser,
                        name,
                    });
                    println!();
                }
                Err(e) => {
                    let reason = concise_reason(&e);
                    println!("  Failed: {reason}\n");
                    outcomes.push(Outcome::Skipped {
                        category: Category::Browser,
                        name,
                        reason,
                    });
                }
            },
            Err(e) => {
                let reason = concise_reason(e);
                println!("  Failed: {reason}");
                outcomes.push(Outcome::Skipped {
                    category: Category::Browser,
                    name,
                    reason,
                });
            }
        }
    }

    // Shells
    for shell in crate::shell::SHELLS {
        if !is_detected(Category::Shell, shell.name()) {
            continue;
        }
        any_detected = true;
        let desc = integration_description(Category::Shell, shell.name());
        match prompt_yn(desc) {
            Some(true) => {}
            Some(false) => continue,
            None => return Ok(()),
        }
        let name = shell.name().to_string();
        match shell.install_hooks(&bin_cmd) {
            Ok(()) => {
                if let Err(e) = shell.install_completions(&bin_cmd) {
                    tracing::warn!("shell {name}: completions failed: {e}");
                }
                outcomes.push(Outcome::Installed {
                    category: Category::Shell,
                    name,
                });
                println!();
            }
            Err(e) => {
                let reason = concise_reason(&e);
                println!("  Failed: {reason}\n");
                outcomes.push(Outcome::Skipped {
                    category: Category::Shell,
                    name,
                    reason,
                });
            }
        }
    }

    if !any_detected {
        anyhow::bail!("no supported integrations detected on this system");
    }

    // Offer to download the default transcription model.
    prompt_model_download();

    // Summary
    let installed_count = outcomes.iter().filter(|o| o.is_installed()).count();
    if installed_count > 0 {
        println!(
            "{installed_count} integration{} installed.",
            if installed_count == 1 { "" } else { "s" }
        );
    } else if outcomes.is_empty() {
        println!("Nothing to install.");
    }

    // Save metadata.
    save_outcomes_meta(&outcomes, &project, dev);

    Ok(())
}

/// Non-interactive install: try every known integration, report results.
fn install_noninteractive(project: Option<Utf8PathBuf>, dev: bool) -> anyhow::Result<()> {
    let bin_cmd = crate::agent::resolve_bin_cmd(dev)?;
    let mut outcomes: Vec<Outcome> = Vec::new();

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

    for shell in crate::shell::SHELLS {
        let name = shell.name().to_string();
        match shell.install_hooks(&bin_cmd) {
            Ok(()) => {
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

    // Download the default transcription model (non-interactive: no prompt).
    download_default_model();

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

    save_outcomes_meta(&outcomes, &project, dev);

    Ok(())
}

/// Save install metadata from a list of outcomes.
fn save_outcomes_meta(outcomes: &[Outcome], project: &Option<Utf8PathBuf>, dev: bool) {
    let mut meta = crate::state::installed_meta().unwrap_or_default();
    meta.version = env!("CARGO_PKG_VERSION").to_string();
    meta.dev = dev;
    for outcome in outcomes {
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
    if let Some(p) = project
        && !meta.project_paths.contains(p)
    {
        meta.project_paths.push(p.clone());
    }
    crate::state::save_install_meta(&meta);
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

/// Resolve the configured engine and default model path.
fn resolve_default_engine() -> (crate::narrate::transcribe::Engine, camino::Utf8PathBuf) {
    use crate::narrate::transcribe::Engine;

    let cwd = camino::Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
        .unwrap_or_default();
    let config = crate::config::Config::load(&cwd);
    let engine = config.engine.unwrap_or(Engine::Parakeet);
    let model_path = config.model.unwrap_or_else(|| engine.default_model_path());
    (engine, model_path)
}

/// Prompt to download the default transcription model (interactive install).
fn prompt_model_download() {
    let (engine, model_path) = resolve_default_engine();
    if engine.is_model_cached(&model_path) {
        return;
    }

    let desc = format!(
        "Download {} transcription model ({})",
        engine.display_name(),
        engine.approx_download_size(),
    );
    match prompt_yn(&desc) {
        Some(true) => {
            if let Err(e) = super::model::download_with_progress(engine, &model_path) {
                println!("  Model download failed: {}\n", concise_reason(&e));
            }
        }
        Some(false) => {
            println!(
                "  Skipped. You can download later with: attend narrate model download\n"
            );
        }
        None => {}
    }
}

/// Download the default transcription model without prompting (non-interactive install).
fn download_default_model() {
    let (engine, model_path) = resolve_default_engine();
    if engine.is_model_cached(&model_path) {
        return;
    }

    if let Err(e) = super::model::download_with_progress(engine, &model_path) {
        eprintln!("Model download failed: {}", concise_reason(&e));
    }
}

#[cfg(test)]
mod tests;
