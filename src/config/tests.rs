use super::*;

/// Loading config from a nonexistent path returns defaults (empty include_dirs).
#[test]
fn load_missing_dir() {
    let config = Config::load(Utf8Path::new("/nonexistent/path"));
    assert!(config.include_dirs.is_empty());
}

/// Loading a nonexistent config file returns None.
#[test]
fn load_file_missing() {
    assert!(load_file(Path::new("/nonexistent/config.toml")).is_none());
}

/// A valid TOML file with include_dirs is parsed correctly.
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

/// An empty TOML file deserializes to defaults (all fields None/empty).
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

/// Engine and model fields are parsed from TOML.
#[test]
fn load_file_engine_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "engine = \"whisper\"\nmodel = \"/custom/model\"\n").unwrap();
    let config = load_file(&path).unwrap();
    assert_eq!(config.engine, Some(Engine::Whisper));
    assert_eq!(config.model, Some(Utf8PathBuf::from("/custom/model")));
}

/// When parent and child both set engine, the closest (child) wins.
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

/// Hierarchical config walk merges include_dirs from both child and parent.
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
        archive_retention: None,
        clipboard_capture: None,
        daemon_idle_timeout: None,
    };
    let other = Config {
        include_dirs: vec![Utf8PathBuf::from("/b")],
        engine: Some(Engine::Parakeet),
        model: Some(Utf8PathBuf::from("/model")),
        silence_duration: Some("3s".to_string()),
        archive_retention: Some("30d".to_string()),
        clipboard_capture: Some(false),
        daemon_idle_timeout: Some("10m".to_string()),
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
    assert_eq!(base.silence_duration, Some("3s".to_string()));
    assert_eq!(base.archive_retention, Some("30d".to_string()));
    // None is filled from other
    assert_eq!(base.daemon_idle_timeout, Some("10m".to_string()));
}

/// retention_duration parses human-friendly strings and defaults to 7 days.
#[test]
fn retention_duration_parsing() {
    use std::time::Duration;

    // Default (None) → 7 days
    let config = Config::default();
    assert_eq!(
        config.retention_duration(),
        Some(Duration::from_secs(7 * 24 * 3600))
    );

    // Explicit duration
    let config = Config {
        archive_retention: Some("24h".to_string()),
        ..Config::default()
    };
    assert_eq!(
        config.retention_duration(),
        Some(Duration::from_secs(24 * 3600))
    );

    // "forever" → None (cleanup disabled)
    let config = Config {
        archive_retention: Some("forever".to_string()),
        ..Config::default()
    };
    assert_eq!(config.retention_duration(), None);
}

/// idle_timeout parses human-friendly strings and defaults to 5 minutes.
#[test]
fn idle_timeout_parsing() {
    use std::time::Duration;

    // Default (None) → 5 minutes
    let config = Config::default();
    assert_eq!(config.idle_timeout(), Some(Duration::from_secs(5 * 60)));

    // Explicit duration
    let config = Config {
        daemon_idle_timeout: Some("10m".to_string()),
        ..Config::default()
    };
    assert_eq!(config.idle_timeout(), Some(Duration::from_secs(10 * 60)));

    // "forever" → None (never auto-exit)
    let config = Config {
        daemon_idle_timeout: Some("forever".to_string()),
        ..Config::default()
    };
    assert_eq!(config.idle_timeout(), None);
}

/// A default Config has clipboard_capture effectively true (None = default on).
#[test]
fn clipboard_capture_defaults_to_true() {
    let config = Config::default();
    // None means "use default" which is true.
    assert!(config.clipboard_capture.unwrap_or(true));
}

/// Parsing clipboard_capture = false from TOML yields false.
#[test]
fn clipboard_capture_explicit_false() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "clipboard_capture = false\n").unwrap();
    let config = load_file(&path).unwrap();
    assert_eq!(config.clipboard_capture, Some(false));
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

// ── Config::merge coverage ──────────────────────────────────────────────────

/// Helper: build a Config with every field populated.
fn fully_populated() -> Config {
    Config {
        include_dirs: vec![Utf8PathBuf::from("/populated/dir")],
        engine: Some(Engine::Whisper),
        model: Some(Utf8PathBuf::from("/populated/model")),
        silence_duration: Some("2500ms".to_string()),
        archive_retention: Some("14d".to_string()),
        clipboard_capture: Some(true),
        daemon_idle_timeout: Some("8m".to_string()),
    }
}

/// Scalar fields use first-wins semantics: when both configs set a scalar,
/// the existing (self) value is preserved.
#[test]
fn merge_scalar_first_wins() {
    let mut a = Config {
        engine: Some(Engine::Whisper),
        model: Some(Utf8PathBuf::from("/a/model")),
        silence_duration: Some("5s".to_string()),
        archive_retention: Some("7d".to_string()),
        clipboard_capture: Some(true),
        daemon_idle_timeout: Some("5m".to_string()),
        ..Config::default()
    };
    let b = Config {
        engine: Some(Engine::Parakeet),
        model: Some(Utf8PathBuf::from("/b/model")),
        silence_duration: Some("10s".to_string()),
        archive_retention: Some("30d".to_string()),
        clipboard_capture: Some(false),
        daemon_idle_timeout: Some("15m".to_string()),
        ..Config::default()
    };
    a.merge(b);
    assert_eq!(a.engine, Some(Engine::Whisper));
    assert_eq!(a.model, Some(Utf8PathBuf::from("/a/model")));
    assert_eq!(a.silence_duration, Some("5s".to_string()));
    assert_eq!(a.archive_retention, Some("7d".to_string()));
    assert_eq!(a.clipboard_capture, Some(true));
    assert_eq!(a.daemon_idle_timeout, Some("5m".to_string()));
}

/// Scalar fields with None in self are filled from other.
#[test]
fn merge_scalar_fills_none() {
    let mut a = Config {
        engine: None,
        model: None,
        silence_duration: None,
        archive_retention: None,
        clipboard_capture: None,
        daemon_idle_timeout: None,
        ..Config::default()
    };
    let b = Config {
        engine: Some(Engine::Parakeet),
        model: Some(Utf8PathBuf::from("/b/model")),
        silence_duration: Some("3s".to_string()),
        archive_retention: Some("30d".to_string()),
        clipboard_capture: Some(false),
        daemon_idle_timeout: Some("10m".to_string()),
        ..Config::default()
    };
    a.merge(b);
    assert_eq!(a.engine, Some(Engine::Parakeet));
    assert_eq!(a.model, Some(Utf8PathBuf::from("/b/model")));
    assert_eq!(a.silence_duration, Some("3s".to_string()));
    assert_eq!(a.archive_retention, Some("30d".to_string()));
    assert_eq!(a.clipboard_capture, Some(false));
    assert_eq!(a.daemon_idle_timeout, Some("10m".to_string()));
}

/// Vector fields are concatenated with self's items first, then other's.
#[test]
fn merge_array_concatenation_order() {
    let mut a = Config {
        include_dirs: vec![Utf8PathBuf::from("/a")],
        ..Config::default()
    };
    let b = Config {
        include_dirs: vec![Utf8PathBuf::from("/b")],
        ..Config::default()
    };
    a.merge(b);
    assert_eq!(
        a.include_dirs,
        vec![Utf8PathBuf::from("/a"), Utf8PathBuf::from("/b")]
    );
}

/// Merging three layers preserves first-wins for scalars across all layers
/// and accumulates arrays in merge order.
#[test]
fn merge_three_layers() {
    let mut a = Config {
        include_dirs: vec![Utf8PathBuf::from("/a")],
        engine: Some(Engine::Whisper),
        model: None,
        silence_duration: None,
        archive_retention: None,
        clipboard_capture: None,
        daemon_idle_timeout: None,
    };
    let b = Config {
        include_dirs: vec![Utf8PathBuf::from("/b")],
        engine: Some(Engine::Parakeet),
        model: Some(Utf8PathBuf::from("/b/model")),
        silence_duration: Some("3s".to_string()),
        archive_retention: None,
        clipboard_capture: None,
        daemon_idle_timeout: Some("8m".to_string()),
    };
    let c = Config {
        include_dirs: vec![Utf8PathBuf::from("/c")],
        engine: Some(Engine::Parakeet),
        model: Some(Utf8PathBuf::from("/c/model")),
        silence_duration: Some("10s".to_string()),
        archive_retention: Some("90d".to_string()),
        clipboard_capture: Some(false),
        daemon_idle_timeout: Some("20m".to_string()),
    };

    // Merge C into B, then B into A (simulates hierarchical walk:
    // A = closest, B = mid, C = farthest).
    a.merge(b);
    a.merge(c);

    // Scalars: first-wins across three layers
    assert_eq!(a.engine, Some(Engine::Whisper), "engine: A wins");
    assert_eq!(
        a.model,
        Some(Utf8PathBuf::from("/b/model")),
        "model: B wins (A was None)"
    );
    assert_eq!(
        a.silence_duration,
        Some("3s".to_string()),
        "silence_duration: B wins"
    );
    assert_eq!(
        a.archive_retention,
        Some("90d".to_string()),
        "archive_retention: C wins (A and B were None)"
    );
    assert_eq!(
        a.clipboard_capture,
        Some(false),
        "clipboard_capture: C wins (A and B were None)"
    );
    assert_eq!(
        a.daemon_idle_timeout,
        Some("8m".to_string()),
        "daemon_idle_timeout: B wins (A was None)"
    );

    // Arrays: accumulated in merge order (A, then B, then C)
    assert_eq!(
        a.include_dirs,
        vec![
            Utf8PathBuf::from("/a"),
            Utf8PathBuf::from("/b"),
            Utf8PathBuf::from("/c"),
        ]
    );
}

/// Merging a default (empty) config into a fully populated config changes nothing.
#[test]
fn merge_empty_into_populated() {
    let mut populated = fully_populated();
    let empty = Config {
        include_dirs: Vec::new(),
        engine: None,
        model: None,
        silence_duration: None,
        archive_retention: None,
        clipboard_capture: None,
        daemon_idle_timeout: None,
    };
    populated.merge(empty);

    // Scalars unchanged
    assert_eq!(populated.engine, Some(Engine::Whisper));
    assert_eq!(populated.model, Some(Utf8PathBuf::from("/populated/model")));
    assert_eq!(populated.silence_duration, Some("2500ms".to_string()));
    assert_eq!(populated.archive_retention, Some("14d".to_string()));
    assert_eq!(populated.clipboard_capture, Some(true));
    assert_eq!(populated.daemon_idle_timeout, Some("8m".to_string()));
    // Arrays unchanged (nothing to extend with)
    assert_eq!(
        populated.include_dirs,
        vec![Utf8PathBuf::from("/populated/dir")]
    );
}

/// Merging a fully populated config into an empty one fills all scalars
/// and populates arrays.
#[test]
fn merge_populated_into_empty() {
    let mut empty = Config {
        include_dirs: Vec::new(),
        engine: None,
        model: None,
        silence_duration: None,
        archive_retention: None,
        clipboard_capture: None,
        daemon_idle_timeout: None,
    };
    empty.merge(fully_populated());

    // All scalars filled from other
    assert_eq!(empty.engine, Some(Engine::Whisper));
    assert_eq!(empty.model, Some(Utf8PathBuf::from("/populated/model")));
    assert_eq!(empty.silence_duration, Some("2500ms".to_string()));
    assert_eq!(empty.archive_retention, Some("14d".to_string()));
    assert_eq!(empty.clipboard_capture, Some(true));
    assert_eq!(empty.daemon_idle_timeout, Some("8m".to_string()));
    // Arrays populated
    assert_eq!(
        empty.include_dirs,
        vec![Utf8PathBuf::from("/populated/dir")]
    );
}

// ── silence_duration deserialization + method ───────────────────────────────

/// silence_duration as a humantime string is stored verbatim.
#[test]
fn silence_duration_string_value() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "silence_duration = \"5s\"\n").unwrap();
    let config = load_file(&path).unwrap();
    assert_eq!(config.silence_duration, Some("5s".to_string()));
}

/// silence_duration as a legacy float is converted via milliseconds.
/// 2.5 seconds becomes "2500ms", preserving fractional precision.
#[test]
fn silence_duration_float_via_milliseconds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "silence_duration = 2.5\n").unwrap();
    let config = load_file(&path).unwrap();
    assert_eq!(config.silence_duration, Some("2500ms".to_string()));
}

/// silence_duration as an integer is converted to seconds string.
#[test]
fn silence_duration_integer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "silence_duration = 3\n").unwrap();
    let config = load_file(&path).unwrap();
    assert_eq!(config.silence_duration, Some("3s".to_string()));
}

/// Negative float values for silence_duration are rejected during parsing.
#[test]
fn silence_duration_rejects_negative_float() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "silence_duration = -1.0\n").unwrap();
    assert!(load_file(&path).is_none());
}

/// Negative integer values for silence_duration are rejected during parsing.
#[test]
fn silence_duration_rejects_negative_integer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "silence_duration = -5\n").unwrap();
    assert!(load_file(&path).is_none());
}

/// Absent silence_duration defaults to None (field level), and the method
/// returns Some(5s).
#[test]
fn silence_duration_method_default() {
    use std::time::Duration;
    let config = Config::default();
    assert_eq!(config.silence_duration, None);
    assert_eq!(config.silence_duration(), Some(Duration::from_secs(5)));
}

/// silence_duration method parses a humantime string correctly.
#[test]
fn silence_duration_method_parses_string() {
    use std::time::Duration;
    let config = Config {
        silence_duration: Some("3s".to_string()),
        ..Config::default()
    };
    assert_eq!(config.silence_duration(), Some(Duration::from_secs(3)));
}

/// silence_duration of "0s" disables silence splitting (returns None).
#[test]
fn silence_duration_method_zero_disables() {
    let config = Config {
        silence_duration: Some("0s".to_string()),
        ..Config::default()
    };
    assert_eq!(config.silence_duration(), None);
}

/// silence_duration of "0ms" also disables silence splitting (returns None).
#[test]
fn silence_duration_method_zero_ms_disables() {
    let config = Config {
        silence_duration: Some("0ms".to_string()),
        ..Config::default()
    };
    assert_eq!(config.silence_duration(), None);
}

/// A legacy float of 2.5 round-trips through deserialization and the method
/// to produce the correct Duration (2.5 seconds = 2500ms).
#[test]
fn silence_duration_float_roundtrip() {
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "silence_duration = 2.5\n").unwrap();
    let config = load_file(&path).unwrap();
    assert_eq!(config.silence_duration(), Some(Duration::from_millis(2500)));
}

/// Merging a default config is an identity operation: for any config `a`,
/// `a.merge(default_empty)` leaves `a` unchanged (scalars are first-wins
/// so None never overwrites, and extending with an empty vec is a no-op).
mod prop {
    use super::*;
    use proptest::prelude::*;

    fn arb_engine() -> impl Strategy<Value = Option<Engine>> {
        prop_oneof![
            Just(None),
            Just(Some(Engine::Whisper)),
            Just(Some(Engine::Parakeet)),
        ]
    }

    fn arb_opt_path() -> impl Strategy<Value = Option<Utf8PathBuf>> {
        prop_oneof![
            Just(None),
            "[a-z/]{1,20}".prop_map(|s| Some(Utf8PathBuf::from(s))),
        ]
    }

    fn arb_opt_duration() -> impl Strategy<Value = Option<String>> {
        prop_oneof![
            Just(None),
            prop_oneof!["1s", "5s", "10s", "500ms", "2500ms"].prop_map(|s| Some(s.to_string())),
        ]
    }

    fn arb_opt_string() -> impl Strategy<Value = Option<String>> {
        prop_oneof![
            Just(None),
            prop_oneof!["7d", "14d", "30d", "forever"].prop_map(|s| Some(s.to_string())),
        ]
    }

    fn arb_opt_bool() -> impl Strategy<Value = Option<bool>> {
        prop_oneof![Just(None), Just(Some(true)), Just(Some(false)),]
    }

    fn arb_paths() -> impl Strategy<Value = Vec<Utf8PathBuf>> {
        proptest::collection::vec("[a-z/]{1,15}".prop_map(Utf8PathBuf::from), 0..4)
    }

    fn arb_config() -> impl Strategy<Value = Config> {
        (
            arb_paths(),
            arb_engine(),
            arb_opt_path(),
            arb_opt_duration(),
            arb_opt_string(),
            arb_opt_bool(),
            arb_opt_string(),
        )
            .prop_map(
                |(
                    include_dirs,
                    engine,
                    model,
                    silence_duration,
                    archive_retention,
                    clipboard_capture,
                    daemon_idle_timeout,
                )| {
                    Config {
                        include_dirs,
                        engine,
                        model,
                        silence_duration,
                        archive_retention,
                        clipboard_capture,
                        daemon_idle_timeout,
                    }
                },
            )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        /// Merging an empty config (all None, empty vecs) into any config is
        /// an identity operation: no scalar changes, no array growth.
        #[test]
        fn merge_idempotent_with_empty(a in arb_config()) {
            let empty = Config {
                include_dirs: Vec::new(),
                engine: None,
                model: None,
                silence_duration: None,
                archive_retention: None,
                clipboard_capture: None,
                daemon_idle_timeout: None,
            };

            // Snapshot before merge
            let dirs_before = a.include_dirs.clone();
            let engine_before = a.engine;
            let model_before = a.model.clone();
            let silence_before = a.silence_duration.clone();
            let retention_before = a.archive_retention.clone();
            let clipboard_before = a.clipboard_capture;
            let idle_before = a.daemon_idle_timeout.clone();

            let mut a = a;
            a.merge(empty);

            prop_assert_eq!(&a.include_dirs, &dirs_before, "include_dirs changed");
            prop_assert_eq!(a.engine, engine_before, "engine changed");
            prop_assert_eq!(&a.model, &model_before, "model changed");
            prop_assert_eq!(&a.silence_duration, &silence_before, "silence_duration changed");
            prop_assert_eq!(&a.archive_retention, &retention_before, "archive_retention changed");
            prop_assert_eq!(a.clipboard_capture, clipboard_before, "clipboard_capture changed");
            prop_assert_eq!(&a.daemon_idle_timeout, &idle_before, "daemon_idle_timeout changed");
        }
    }
}
