//! End-to-end benchmark: measures wall-clock time from process spawn to exit.
//!
//! Invokes the release binary directly via `std::process::Command` (no shell),
//! piping stdin JSON and capturing stdout. This is the same path Claude Code's
//! hook runner takes, minus the shell overhead.
//!
//! Run:
//!   cargo bench --bench e2e
//!
//! The benchmark builds in release mode automatically (Cargo compiles benchmarks
//! with --release). The binary under test is the same release artifact.

use std::process::{Command, Stdio};

use criterion::{Criterion, criterion_group, criterion_main};

/// Locate the binary built by Cargo. In bench profile the binary lives next to
/// the benchmark runner under `target/release/`.
fn binary() -> std::path::PathBuf {
    // `CARGO_BIN_EXE_attend` is only set for integration tests, not benchmarks.
    // Fall back to building the path manually from the deps directory.
    let mut path = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("parent")
        .to_path_buf();
    // Benchmark binaries are in target/release/deps/; the main binary is one
    // level up in target/release/.
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("attend");
    assert!(
        path.exists(),
        "release binary not found at {}; run `cargo build --release` first",
        path.display()
    );
    path
}

fn bench_user_prompt(c: &mut Criterion) {
    let bin = binary();
    let cwd = std::env::current_dir().expect("cwd");
    let stdin_json = serde_json::json!({
        "session_id": "bench",
        "cwd": cwd,
    })
    .to_string();

    c.bench_function("hook user-prompt --agent claude", |b| {
        b.iter(|| {
            let mut child = Command::new(&bin)
                .args(["hook", "user-prompt", "--agent", "claude"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn");

            // Write stdin and close it so the child can proceed.
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                stdin.write_all(stdin_json.as_bytes()).expect("write stdin");
            }

            let output = child.wait_with_output().expect("wait");
            assert!(output.status.success(), "exit: {}", output.status);
        });
    });
}

fn bench_default_human(c: &mut Criterion) {
    let bin = binary();

    c.bench_function("glance (human)", |b| {
        b.iter(|| {
            let output = Command::new(&bin)
                .args(["glance"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .expect("run");
            assert!(output.status.success(), "exit: {}", output.status);
        });
    });
}

fn bench_default_json(c: &mut Criterion) {
    let bin = binary();

    c.bench_function("glance (json)", |b| {
        b.iter(|| {
            let output = Command::new(&bin)
                .args(["glance", "--format", "json"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .expect("run");
            assert!(output.status.success(), "exit: {}", output.status);
        });
    });
}

criterion_group!(
    benches,
    bench_user_prompt,
    bench_default_human,
    bench_default_json
);
criterion_main!(benches);
