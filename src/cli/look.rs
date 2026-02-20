//! Handler for the `look` subcommand.

use camino::Utf8PathBuf;

use super::Format;

/// Run the look subcommand (one-shot or watch mode).
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    dir: Option<Utf8PathBuf>,
    format: Format,
    full: bool,
    before: Option<usize>,
    after: Option<usize>,
    watch: bool,
    interval: Option<f64>,
    args: Vec<String>,
) -> anyhow::Result<()> {
    if watch {
        crate::watch::run(
            crate::watch::WatchMode::View,
            dir.as_deref(),
            interval,
            &format,
            full,
            before,
            after,
        )
    } else {
        let entries = if args.is_empty() {
            match crate::state::EditorState::current(dir.as_deref(), &[])? {
                Some(state) => state.files,
                None => return Ok(()),
            }
        } else if args.len() == 1 && args[0] == "-" {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
            crate::view::parse_compact(&input)?
        } else {
            crate::view::parse_compact(&args.join(" "))?
        };
        let extent = if full {
            crate::view::Extent::Full
        } else if before.is_some() || after.is_some() {
            crate::view::Extent::Lines {
                before: before.unwrap_or(0),
                after: after.unwrap_or(0),
            }
        } else {
            crate::view::Extent::Exact
        };
        match format {
            Format::Human => {
                print!("{}", crate::view::render(&entries, dir.as_deref(), extent)?);
            }
            Format::Json => {
                let payload = crate::view::render_json(&entries, dir.as_deref(), extent)?;
                let wrapped = crate::util::Timestamped::now(payload);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&wrapped).expect("serialization of known type")
                );
            }
        }
        Ok(())
    }
}
