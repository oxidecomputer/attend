use super::*;

#[test]
fn load_missing_dir() {
    let config = Config::load(Path::new("/nonexistent/path"));
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
    let raw = load_file(&path).unwrap();
    assert_eq!(
        raw.include_dirs,
        vec![PathBuf::from("/Users/oxide/src/shared")]
    );
}

#[test]
fn load_file_empty_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "").unwrap();
    let raw = load_file(&path).unwrap();
    assert!(raw.include_dirs.is_empty());
    assert!(raw.engine.is_none());
    assert!(raw.model.is_none());
}

#[test]
fn load_file_engine_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "engine = \"whisper\"\nmodel = \"/custom/model\"\n").unwrap();
    let raw = load_file(&path).unwrap();
    assert_eq!(raw.engine.as_deref(), Some("whisper"));
    assert_eq!(raw.model, Some(PathBuf::from("/custom/model")));
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

    let config = Config::load(&child);
    // Child (closest) wins
    assert!(matches!(config.engine, Some(Engine::Parakeet)));
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

    let config = Config::load(&child);
    // Child config should come first (closer), then parent
    assert!(config.include_dirs.contains(&PathBuf::from("/child/lib")));
    assert!(config.include_dirs.contains(&PathBuf::from("/parent/lib")));
}
