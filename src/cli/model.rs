use camino::Utf8PathBuf;
use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::Config;
use crate::narrate::transcribe::Engine;

/// Model management subcommands.
#[derive(Subcommand)]
pub enum ModelCommand {
    /// Download the transcription model.
    Download {
        /// Transcription engine (defaults to config, then parakeet).
        #[arg(long)]
        engine: Option<Engine>,
        /// Custom model path (overrides default cache location).
        #[arg(long)]
        model_path: Option<Utf8PathBuf>,
    },
}

impl ModelCommand {
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            ModelCommand::Download { engine, model_path } => download(engine, model_path),
        }
    }
}

fn download(engine_arg: Option<Engine>, path_arg: Option<Utf8PathBuf>) -> anyhow::Result<()> {
    let cwd = Utf8PathBuf::try_from(std::env::current_dir()?)?;
    let config = Config::load(&cwd);

    let engine = engine_arg.or(config.engine).unwrap_or(Engine::Parakeet);
    let model_path = path_arg
        .or(config.model)
        .unwrap_or_else(|| engine.default_model_path());

    if engine.is_model_cached(&model_path) {
        println!(
            "{} model already downloaded at {model_path}",
            engine.display_name()
        );
        return Ok(());
    }

    download_with_progress(engine, &model_path)
}

/// Download the model for `engine` to `model_path`, showing a progress bar
/// per file. Returns `Ok(())` if the model was already cached.
///
/// Used by both `attend narrate model download` and `attend install`.
pub fn download_with_progress(engine: Engine, model_path: &Utf8PathBuf) -> anyhow::Result<()> {
    if engine.is_model_cached(model_path) {
        return Ok(());
    }

    println!(
        "Downloading {} model to {model_path}",
        engine.display_name()
    );

    // Track which file's progress bar is active. When the callback fires
    // with a new filename we finish the old bar and create a new one.
    let mut current_file: Option<String> = None;
    let mut bar: Option<ProgressBar> = None;

    let style = ProgressStyle::with_template(
        "  {bar:40.cyan/dim} {bytes}/{total_bytes} {bytes_per_sec} eta {eta}  {msg}",
    )
    .expect("valid template")
    .progress_chars("━╸─");

    let spinner_style = ProgressStyle::with_template("  {spinner} {bytes} {bytes_per_sec}  {msg}")
        .expect("valid template");

    engine.ensure_model_with_progress(model_path, &mut |filename, bytes, total| {
        // New file — finish previous bar and start a new one.
        if current_file.as_deref() != Some(filename) {
            if let Some(prev) = bar.take() {
                prev.finish();
            }
            let pb = if let Some(len) = total {
                ProgressBar::new(len).with_style(style.clone())
            } else {
                ProgressBar::new_spinner().with_style(spinner_style.clone())
            };
            pb.set_message(filename.to_string());
            current_file = Some(filename.to_string());
            bar = Some(pb);
        }

        if let Some(pb) = &bar {
            pb.set_position(bytes);
        }
    })?;

    if let Some(pb) = bar.take() {
        pb.finish();
    }

    println!("{} model downloaded successfully.", engine.display_name());
    Ok(())
}
