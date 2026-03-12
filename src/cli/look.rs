//! Handler for the `look` subcommand.

use camino::Utf8PathBuf;

use super::Format;
use crate::state::FileEntry;

/// Arguments for the `look` subcommand.
#[derive(clap::Args)]
pub struct LookArgs {
    /// Resolve paths relative to this directory and show relative paths.
    #[arg(long, short)]
    pub dir: Option<Utf8PathBuf>,

    /// Output format.
    #[arg(long, short, default_value = "human")]
    pub format: Format,

    /// Show entire file contents with highlights inline.
    #[arg(long, conflicts_with_all = ["before", "after"])]
    pub full: bool,

    /// Context lines before each excerpt.
    #[arg(long, short = 'B')]
    pub before: Option<usize>,

    /// Context lines after each excerpt.
    #[arg(long, short = 'A')]
    pub after: Option<usize>,

    /// Continuously watch editor state in view mode.
    #[arg(long, short)]
    pub watch: bool,

    /// Override polling / debounce interval in seconds.
    #[arg(long, short = 'i')]
    pub interval: Option<f64>,

    /// File paths and positions in compact format (same as default output).
    /// E.g.: src/foo.rs 5:12 19:40-24:6 src/bar.rs 10:1
    #[arg(trailing_var_arg = true)]
    pub args: Vec<String>,
}

impl LookArgs {
    /// Run the look subcommand (one-shot or watch mode).
    pub fn run(self) -> anyhow::Result<()> {
        if self.watch {
            crate::watch::run(
                crate::watch::WatchMode::View,
                self.dir.as_deref(),
                self.interval,
                &self.format,
                self.full,
                self.before,
                self.after,
                crate::clock::process_clock().for_thread(),
            )
        } else {
            let entries = if self.args.is_empty() {
                match crate::state::EditorState::current(self.dir.as_deref(), &[])? {
                    Some(state) => state.files,
                    None => return Ok(()),
                }
            } else if self.args.len() == 1 && self.args[0] == "-" {
                let mut input = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
                crate::view::parse_compact(&input)?
            } else {
                crate::view::parse_compact(&self.args.join(" "))?
            };
            let entries = filter_to_scope(entries);
            let extent = if self.full {
                crate::view::Extent::Full
            } else if self.before.is_some() || self.after.is_some() {
                crate::view::Extent::Lines {
                    before: self.before.unwrap_or(0),
                    after: self.after.unwrap_or(0),
                }
            } else {
                crate::view::Extent::Exact
            };
            match self.format {
                Format::Human => {
                    print!(
                        "{}",
                        crate::view::render(&entries, self.dir.as_deref(), extent)?
                    );
                }
                Format::Json => {
                    let payload = crate::view::render_json(&entries, self.dir.as_deref(), extent)?;
                    let wrapped =
                        crate::util::Timestamped::at(crate::clock::process_clock().now(), payload);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&wrapped)
                            .expect("serialization of known type")
                    );
                }
            }
            Ok(())
        }
    }
}

/// Filter file entries to the project scope (cwd + include_dirs) and
/// relativize paths to the project root.
///
/// Uses the process's current working directory and the attend config to
/// determine scope, matching the same filtering the narration pipeline applies.
fn filter_to_scope(entries: Vec<FileEntry>) -> Vec<FileEntry> {
    let cwd = Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
        .unwrap_or_else(|_| Utf8PathBuf::from("."));
    let config = crate::config::Config::load(&cwd);

    entries
        .into_iter()
        .filter(|e| crate::util::path_included(e.path.as_str(), &cwd, &config.include_dirs))
        .map(|mut e| {
            if let Ok(rel) = camino::Utf8Path::new(&e.path).strip_prefix(&cwd) {
                e.path = rel.to_string().into();
            }
            e
        })
        .collect()
}
