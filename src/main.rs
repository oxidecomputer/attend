use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;

// --- CLI ---

#[derive(Parser)]
#[command(name = "zed-context", about = "Read Zed editor state")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Filter to files under this directory (defaults to $PWD)
    #[arg(long, global = true)]
    cwd: Option<PathBuf>,

    /// Output format
    #[arg(long, default_value = "human")]
    format: Format,
}

#[derive(Clone, ValueEnum)]
enum Format {
    Human,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Hook mode for agent integration
    Hook {
        /// Agent to target
        #[arg(long)]
        agent: Option<String>,

        /// Run as session-start hook (clear cache, emit instructions)
        #[arg(long)]
        session_start: bool,

        #[command(subcommand)]
        sub: Option<HookSub>,
    },
}

#[derive(Subcommand)]
enum HookSub {
    /// Install hooks into agent settings
    Install {
        /// Agent to install for
        #[arg(long)]
        agent: String,

        /// Project path (installs to <path>/.claude/settings.json instead of global)
        #[arg(long)]
        project: Option<PathBuf>,

        /// Use absolute path to current binary (for development)
        #[arg(long)]
        dev: bool,
    },
    /// Remove hooks from agent settings
    Uninstall {
        /// Agent to uninstall for
        #[arg(long)]
        agent: String,

        /// Project path (removes from <path>/.claude/settings.json instead of global)
        #[arg(long)]
        project: Option<PathBuf>,
    },
}

// --- Data model ---

#[derive(Debug, Serialize)]
struct EditorState {
    panes: Vec<PaneEntry>,
}

#[derive(Debug, Serialize)]
struct PaneEntry {
    path: String,
    cursors: Vec<Position>,
    selections: Vec<Selection>,
}

#[derive(Debug, Clone, Serialize)]
struct Position {
    line: usize,
    col: usize,
}

#[derive(Debug, Serialize)]
struct Selection {
    start: Position,
    end: Position,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

// --- DB discovery ---

fn find_zed_db() -> Option<PathBuf> {
    let data_dir = dirs::data_dir()?;
    let zed_db_dir = data_dir.join("Zed").join("db");

    let mut candidates: Vec<PathBuf> = fs::read_dir(&zed_db_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("0-")))
        .map(|e| e.path().join("db.sqlite"))
        .filter(|p| p.exists())
        .collect();

    // Pick the one with the most recently modified WAL
    candidates.sort_by(|a, b| {
        let wal_mtime = |p: &Path| {
            let wal = p.with_extension("sqlite-wal");
            fs::metadata(&wal).and_then(|m| m.modified()).ok()
        };
        wal_mtime(b).cmp(&wal_mtime(a))
    });

    candidates.into_iter().next()
}

// --- DB query ---

struct RawEditor {
    pane_id: i64,
    path: String,
    sel_start: Option<i64>,
    sel_end: Option<i64>,
}

fn query_editors(db_path: &Path) -> Result<Vec<RawEditor>, String> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("failed to open DB: {e}"))?;

    // Enable WAL mode reading
    conn.pragma_update(None, "journal_mode", "wal")
        .map_err(|e| format!("failed to set journal_mode: {e}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT i.pane_id, e.path, es.start, es.end \
             FROM items i \
             JOIN editors e ON i.item_id = e.item_id AND i.workspace_id = e.workspace_id \
             LEFT JOIN editor_selections es \
               ON e.item_id = es.editor_id AND e.workspace_id = es.workspace_id \
             WHERE i.kind = 'Editor' AND i.active = 1 \
             ORDER BY i.pane_id, e.path, es.start",
        )
        .map_err(|e| format!("prepare failed: {e}"))?;

    let editors: Vec<RawEditor> = stmt
        .query_map([], |row| {
            let path_bytes: Vec<u8> = row.get(1)?;
            let path = String::from_utf8(path_bytes).unwrap_or_default();
            Ok(RawEditor {
                pane_id: row.get(0)?,
                path,
                sel_start: row.get(2)?,
                sel_end: row.get(3)?,
            })
        })
        .map_err(|e| format!("query failed: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(editors)
}

// --- Byte offset to line:col ---

fn byte_offset_to_position(content: &[u8], offset: usize) -> Position {
    let offset = offset.min(content.len());
    let mut line = 1;
    let mut col = 1;
    for &b in &content[..offset] {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    Position { line, col }
}

// --- Build structured state ---

fn build_editor_state(raw_editors: Vec<RawEditor>, cwd: Option<&Path>) -> EditorState {
    let cwd_str = cwd.map(|p| p.to_string_lossy().to_string());

    // Group by (pane_id, path), preserving order via BTreeMap
    let mut pane_files: BTreeMap<(i64, String), Vec<(i64, i64)>> = BTreeMap::new();
    for ed in &raw_editors {
        if let Some(ref cwd) = cwd_str {
            if !ed.path.starts_with(cwd.as_str()) {
                continue;
            }
        }
        let entry = pane_files
            .entry((ed.pane_id, ed.path.clone()))
            .or_default();
        if let (Some(start), Some(end)) = (ed.sel_start, ed.sel_end) {
            entry.push((start, end));
        }
    }

    let mut panes = Vec::new();
    for ((_pane_id, path), selections) in &pane_files {
        let rel_path = if let Some(ref cwd) = cwd_str {
            if let Some(stripped) = path.strip_prefix(&format!("{cwd}/")) {
                stripped.to_string()
            } else if let Some(stripped) = path.strip_prefix(cwd.as_str()) {
                stripped.trim_start_matches('/').to_string()
            } else {
                path.clone()
            }
        } else {
            path.clone()
        };

        // Read file content for offset conversion (only needed if there are selections)
        let content = if !selections.is_empty() {
            match fs::read(path) {
                Ok(c) => Some(c),
                Err(_) => None,
            }
        } else {
            None
        };

        let mut cursors = Vec::new();
        let mut sels = Vec::new();
        if let Some(ref content) = content {
            for &(start, end) in selections {
                let start_pos = byte_offset_to_position(content, start as usize);
                if start == end {
                    cursors.push(start_pos);
                } else {
                    let end_pos = byte_offset_to_position(content, end as usize);
                    sels.push(Selection {
                        start: start_pos,
                        end: end_pos,
                    });
                }
            }
        }

        panes.push(PaneEntry {
            path: rel_path,
            cursors,
            selections: sels,
        });
    }

    EditorState { panes }
}

// --- Output formatting ---

fn format_human(state: &EditorState) -> String {
    let parts: Vec<String> = state
        .panes
        .iter()
        .map(|p| {
            let mut positions: Vec<String> = Vec::new();
            for c in &p.cursors {
                positions.push(format!("{c}"));
            }
            for sel in &p.selections {
                positions.push(format!("{}-{}", sel.start, sel.end));
            }
            if positions.is_empty() {
                p.path.clone()
            } else {
                format!("{} {}", p.path, positions.join(","))
            }
        })
        .collect();
    parts.join(" | ")
}

fn format_json(state: &EditorState) -> String {
    serde_json::to_string_pretty(state).unwrap_or_default()
}

// --- Core logic: get state as human string ---

fn get_state(cwd: &Path) -> Option<(EditorState, String)> {
    let db_path = find_zed_db()?;
    let raw_editors = query_editors(&db_path).ok()?;
    let state = build_editor_state(raw_editors, Some(cwd));
    if state.panes.is_empty() {
        return None;
    }
    let human = format_human(&state);
    Some((state, human))
}

// --- Hook: caching ---

fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("zed-context"))
}

fn cache_path(session_id: &str) -> Option<PathBuf> {
    Some(cache_dir()?.join(format!("cache-{session_id}.txt")))
}

fn read_stdin_json() -> Option<serde_json::Value> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

fn run_hook(agent: &str, session_start: bool, cli_cwd: Option<PathBuf>) {
    if agent != "claude" {
        eprintln!("unsupported agent: {agent}");
        process::exit(1);
    }

    let stdin_json = read_stdin_json();
    let session_id = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let stdin_cwd = stdin_json
        .as_ref()
        .and_then(|v| v.get("cwd"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from);

    let cwd = cli_cwd
        .or(stdin_cwd)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if session_start {
        // Delete cache file
        if let Some(sid) = &session_id
            && let Some(cp) = cache_path(sid)
        {
            let _ = fs::remove_file(cp);
        }
        // Emit instructions
        print!(concat!(
            "<zed-context-instructions>\n",
            "Messages in <zed-context> tags show the user's current Zed editor state. ",
            "They are injected automatically \u{2014} the user does not see them. ",
            "Do NOT acknowledge or respond to them directly. ",
            "Use them silently to understand what the user is looking at. ",
            "Read files to see content at those locations.\n",
            "\n",
            "Format: <zed-context><path> <pos>[,<pos>]... ",
            "[| <next pane>...]</zed-context>\n",
            "\n",
            "Paths are relative to the project root. ",
            "Each <pos> is line:col (cursor) or line:col-line:col (selection). ",
            "Multiple positions are comma-separated. ",
            "Multiple panes are separated by |.\n",
            "</zed-context-instructions>",
        ));
        return;
    }

    // Normal hook: get state, compare to cache
    let human = match get_state(&cwd) {
        Some((_state, human)) => human,
        None => return,
    };

    if let Some(sid) = &session_id
        && let Some(cp) = cache_path(sid)
    {
        if let Ok(cached) = fs::read_to_string(&cp)
            && cached == human
        {
            return; // unchanged
        }
        // Write new cache
        if let Some(parent) = cp.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&cp, &human);
    }

    println!("<zed-context>{human}</zed-context>");
}

// --- Hook install ---

fn run_hook_install(agent: &str, project: Option<PathBuf>, dev: bool) {
    if agent != "claude" {
        eprintln!("unsupported agent: {agent}");
        process::exit(1);
    }

    // Determine binary command
    let bin_name = std::env::args()
        .next()
        .map(|a| {
            Path::new(&a)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_else(|| "zed-context".to_string());

    let bin_cmd = if dev {
        std::env::current_exe()
            .expect("cannot determine current exe path")
            .to_string_lossy()
            .to_string()
    } else {
        // Verify on PATH
        match which::which(&bin_name) {
            Ok(_) => bin_name,
            Err(_) => {
                eprintln!(
                    "error: '{bin_name}' not found on $PATH. \
                     Use --dev to use absolute path instead."
                );
                process::exit(1);
            }
        }
    };

    // Determine settings file path
    let settings_path = if let Some(proj) = project {
        proj.join(".claude").join("settings.json")
    } else {
        dirs::home_dir()
            .expect("cannot determine home directory")
            .join(".claude")
            .join("settings.json")
    };

    // Read existing settings or start fresh
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path).expect("cannot read settings file");
        serde_json::from_str(&content).expect("settings file is not valid JSON")
    } else {
        serde_json::json!({})
    };

    let obj = settings.as_object_mut().expect("settings is not an object");

    // Build hook commands
    let session_start_cmd = format!("{bin_cmd} hook --agent claude --session-start");
    let prompt_cmd = format!("{bin_cmd} hook --agent claude");

    // Build the hooks structure
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().expect("hooks is not an object");

    // SessionStart
    let session_start_hook = serde_json::json!({
        "matcher": "startup|clear|compact",
        "hooks": [
            {
                "type": "command",
                "command": session_start_cmd
            }
        ]
    });

    let ss_arr = hooks_obj
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));
    let ss_vec = ss_arr.as_array_mut().expect("SessionStart is not an array");

    // Remove existing zed-context entries (idempotent)
    ss_vec.retain(|entry| {
        let s = serde_json::to_string(entry).unwrap_or_default();
        !s.contains("zed-context") && !s.contains(&bin_cmd)
    });
    ss_vec.push(session_start_hook);

    // UserPromptSubmit
    let prompt_hook = serde_json::json!({
        "hooks": [
            {
                "type": "command",
                "command": prompt_cmd,
                "timeout": 5
            }
        ]
    });

    let ups_arr = hooks_obj
        .entry("UserPromptSubmit")
        .or_insert_with(|| serde_json::json!([]));
    let ups_vec = ups_arr
        .as_array_mut()
        .expect("UserPromptSubmit is not an array");

    ups_vec.retain(|entry| {
        let s = serde_json::to_string(entry).unwrap_or_default();
        !s.contains("zed-context") && !s.contains(&bin_cmd)
    });
    ups_vec.push(prompt_hook);

    // Write back
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).expect("cannot create settings directory");
    }
    let output = serde_json::to_string_pretty(&settings).expect("cannot serialize settings");
    fs::write(&settings_path, format!("{output}\n")).expect("cannot write settings file");

    println!("Installed hooks to {}", settings_path.display());
}

// --- Hook uninstall ---

fn run_hook_uninstall(agent: &str, project: Option<PathBuf>) {
    if agent != "claude" {
        eprintln!("unsupported agent: {agent}");
        process::exit(1);
    }

    let settings_path = if let Some(proj) = project {
        proj.join(".claude").join("settings.json")
    } else {
        dirs::home_dir()
            .expect("cannot determine home directory")
            .join(".claude")
            .join("settings.json")
    };

    if !settings_path.exists() {
        println!("No settings file found at {}", settings_path.display());
        return;
    }

    let content = fs::read_to_string(&settings_path).expect("cannot read settings file");
    let mut settings: serde_json::Value =
        serde_json::from_str(&content).expect("settings file is not valid JSON");

    let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        println!("No hooks found in {}", settings_path.display());
        return;
    };

    let mut removed = false;
    for key in &["SessionStart", "UserPromptSubmit"] {
        if let Some(arr) = hooks.get_mut(*key).and_then(|v| v.as_array_mut()) {
            let before = arr.len();
            arr.retain(|entry| {
                let s = serde_json::to_string(entry).unwrap_or_default();
                !s.contains("zed-context")
            });
            if arr.len() < before {
                removed = true;
            }
        }
    }

    if removed {
        let output = serde_json::to_string_pretty(&settings).expect("cannot serialize settings");
        fs::write(&settings_path, format!("{output}\n")).expect("cannot write settings file");
        println!("Removed hooks from {}", settings_path.display());
    } else {
        println!("No zed-context hooks found in {}", settings_path.display());
    }
}

// --- Main ---

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Hook {
            agent: _,
            session_start: _,
            sub:
                Some(HookSub::Install {
                    agent: install_agent,
                    project,
                    dev,
                }),
        }) => {
            run_hook_install(&install_agent, project, dev);
        }

        Some(Command::Hook {
            agent: _,
            session_start: _,
            sub:
                Some(HookSub::Uninstall {
                    agent: uninstall_agent,
                    project,
                }),
        }) => {
            run_hook_uninstall(&uninstall_agent, project);
        }

        Some(Command::Hook {
            agent,
            session_start,
            sub: None,
        }) => {
            let agent = agent.unwrap_or_else(|| {
                eprintln!("error: --agent is required for hook mode");
                process::exit(1);
            });
            run_hook(&agent, session_start, cli.cwd);
        }

        None => {
            let db_path = match find_zed_db() {
                Some(p) => p,
                None => return, // silent exit
            };

            let raw_editors = match query_editors(&db_path) {
                Ok(r) => r,
                Err(_) => return, // silent exit
            };

            let state = build_editor_state(raw_editors, cli.cwd.as_deref());
            if state.panes.is_empty() {
                return;
            }

            match cli.format {
                Format::Human => println!("{}", format_human(&state)),
                Format::Json => println!("{}", format_json(&state)),
            }
        }
    }
}
