//! Handler for the `glance` subcommand.

use camino::Utf8PathBuf;

use super::Format;

/// Run the glance subcommand (one-shot or watch mode).
pub(super) fn run(
    dir: Option<Utf8PathBuf>,
    format: Format,
    watch: bool,
    interval: Option<f64>,
) -> anyhow::Result<()> {
    if watch {
        crate::watch::run(
            crate::watch::WatchMode::Compact,
            dir.as_deref(),
            interval,
            &format,
            false,
            None,
            None,
        )
    } else {
        if let Some(state) = crate::state::EditorState::current(dir.as_deref(), &[])? {
            match format {
                Format::Human => println!("{state}"),
                Format::Json => {
                    let payload = crate::state::CompactPayload::from_state(&state);
                    let wrapped = crate::util::Timestamped::now(payload);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&wrapped)
                            .expect("serialization of known type")
                    );
                }
            }
        }
        Ok(())
    }
}
