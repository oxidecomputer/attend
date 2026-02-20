use super::*;

#[test]
fn load_missing_dir() {
    let config = Config::load(Utf8Path::new("/nonexistent/path"));
    assert!(config.include_dirs.is_empty());
}

#[test]
fn load_file_missing() {
    assert!(load_file(Path::new("/nonexistent/config.toml")).is_none());
}

#[test]
fn load_file_valid_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "include_dirs = [\"/Users/oxide/src/shared\"]\n").unwrap();
    let config = load_file(&path).unwrap();
    assert_eq!(
        config.include_dirs,
        vec![Utf8PathBuf::from("/Users/oxide/src/shared")]
    );
}

#[test]
fn load_file_empty_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "").unwrap();
    let config = load_file(&path).unwrap();
    assert!(config.include_dirs.is_empty());
    assert!(config.engine.is_none());
    assert!(config.model.is_none());
}

#[test]
fn load_file_engine_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "engine = \"whisper\"\nmodel = \"/custom/model\"\n").unwrap();
    let config = load_file(&path).unwrap();
    assert_eq!(config.engine, Some(Engine::Whisper));
    assert_eq!(config.model, Some(Utf8PathBuf::from("/custom/model")));
}

#[test]
fn engine_closest_wins() {
    let dir = tempfile::tempdir().unwrap();
    // Parent sets engine = whisper
    let parent_attend = dir.path().join(".attend");
    std::fs::create_dir_all(&parent_attend).unwrap();
    std::fs::write(parent_attend.join("config.toml"), "engine = \"whisper\"\n").unwrap();

    // Child overrides with engine = parakeet
    let child = dir.path().join("child");
    let child_attend = child.join(".attend");
    std::fs::create_dir_all(&child_attend).unwrap();
    std::fs::write(child_attend.join("config.toml"), "engine = \"parakeet\"\n").unwrap();

    let child_utf8 = Utf8PathBuf::try_from(child).unwrap();
    let config = Config::load(&child_utf8);
    // Child (closest) wins
    assert_eq!(config.engine, Some(Engine::Parakeet));
}

#[test]
fn hierarchical_walk() {
    let dir = tempfile::tempdir().unwrap();
    // Create parent config
    let parent_attend = dir.path().join(".attend");
    std::fs::create_dir_all(&parent_attend).unwrap();
    std::fs::write(
        parent_attend.join("config.toml"),
        "include_dirs = [\"/parent/lib\"]\n",
    )
    .unwrap();

    // Create child directory with its own config
    let child = dir.path().join("child");
    let child_attend = child.join(".attend");
    std::fs::create_dir_all(&child_attend).unwrap();
    std::fs::write(
        child_attend.join("config.toml"),
        "include_dirs = [\"/child/lib\"]\n",
    )
    .unwrap();

    let child_utf8 = Utf8PathBuf::try_from(child).unwrap();
    let config = Config::load(&child_utf8);
    // Child config should come first (closer), then parent
    assert!(
        config
            .include_dirs
            .contains(&Utf8PathBuf::from("/child/lib"))
    );
    assert!(
        config
            .include_dirs
            .contains(&Utf8PathBuf::from("/parent/lib"))
    );
}

/// Merge concatenates arrays and applies first-wins for scalars.
#[test]
fn merge_semantics() {
    let mut base = Config {
        include_dirs: vec![Utf8PathBuf::from("/a")],
        engine: Some(Engine::Whisper),
        model: None,
        silence_duration: None,
    };
    let other = Config {
        include_dirs: vec![Utf8PathBuf::from("/b")],
        engine: Some(Engine::Parakeet),
        model: Some(Utf8PathBuf::from("/model")),
        silence_duration: Some(3.0),
    };
    base.merge(other);
    assert_eq!(
        base.include_dirs,
        vec![Utf8PathBuf::from("/a"), Utf8PathBuf::from("/b")]
    );
    // First wins for scalars
    assert_eq!(base.engine, Some(Engine::Whisper));
    // None is filled from other
    assert_eq!(base.model, Some(Utf8PathBuf::from("/model")));
    assert_eq!(base.silence_duration, Some(3.0));
}

/// Unknown engine values in TOML are reported and ignored.
#[test]
fn unknown_engine_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "engine = \"unknown\"\n").unwrap();
    let config = load_file(&path);
    // serde will fail to deserialize an unknown engine variant
    assert!(config.is_none());
}
