/// Every Whisper model name in MODEL_NAMES has a corresponding SHA-256 checksum.
#[test]
fn whisper_model_names_all_have_checksums() {
    for name in super::whisper::MODEL_NAMES {
        assert!(
            super::whisper::expected_checksum(name).is_some(),
            "missing SHA-256 checksum for Whisper model: {name}"
        );
    }
}

/// Every Parakeet model file in MODEL_FILES has a corresponding SHA-256 checksum,
/// except for `vocab.txt` which is a small non-LFS file.
#[test]
fn parakeet_model_files_have_checksums_except_vocab() {
    for name in super::parakeet::MODEL_FILES {
        if *name == "vocab.txt" {
            // vocab.txt is a small non-LFS file; checksum intentionally omitted.
            assert!(
                super::parakeet::expected_checksum(name).is_none(),
                "vocab.txt should not have a checksum (small non-LFS file)"
            );
            continue;
        }
        assert!(
            super::parakeet::expected_checksum(name).is_some(),
            "missing SHA-256 checksum for Parakeet model file: {name}"
        );
    }
}

/// All SHA-256 checksums are valid lowercase hex strings of the correct length (64 chars).
#[test]
fn checksums_are_valid_sha256_hex() {
    let all_checksums: Vec<(&str, &str)> = super::whisper::MODEL_NAMES
        .iter()
        .filter_map(|name| super::whisper::expected_checksum(name).map(|cs| (*name, cs)))
        .chain(
            super::parakeet::MODEL_FILES
                .iter()
                .filter_map(|name| super::parakeet::expected_checksum(name).map(|cs| (*name, cs))),
        )
        .collect();

    assert!(
        !all_checksums.is_empty(),
        "expected at least one checksum to validate"
    );

    for (name, checksum) in &all_checksums {
        assert_eq!(
            checksum.len(),
            64,
            "checksum for {name} has wrong length: {} (expected 64)",
            checksum.len()
        );
        assert!(
            checksum.chars().all(|c| c.is_ascii_hexdigit()),
            "checksum for {name} contains non-hex characters: {checksum}"
        );
        assert_eq!(
            *checksum,
            checksum.to_lowercase(),
            "checksum for {name} should be lowercase hex"
        );
    }
}
