//! phase11s §6.2 — feature-flag gate IT.
//!
//! Pins the contract: the `cortex-claude-archive` bin SHALL refuse to build
//! without `--features claude-archive`. Cargo emits a deterministic error
//! ("the package … requires the features … claude-archive") that this test
//! captures.
//!
//! The IT shells out to `cargo build` with a controlled CARGO_TARGET_DIR
//! so it does not perturb the developer's main `target/`.
//!
//! Skipped automatically (returns Ok) when CARGO is not on PATH (rare on
//! dev machines but possible on minimal CI runners that already pre-built
//! the workspace and don't have cargo in $PATH for the spawned tests).

use std::process::Command;

fn skip_unless_gated() -> bool {
    if std::env::var("CORTEX_FEATURE_GATES_IT").as_deref() != Ok("1") {
        eprintln!("skipping feature_gates_it: set CORTEX_FEATURE_GATES_IT=1 to enable");
        return true;
    }
    false
}

#[test]
fn cortex_claude_archive_bin_requires_feature() {
    if skip_unless_gated() {
        return;
    }
    let cargo = match std::env::var("CARGO") {
        Ok(v) => v,
        Err(_) => "cargo".to_string(),
    };

    // Use a per-test target dir so the harness does not race against the
    // outer cargo invocation that's already holding the main target/ lock.
    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Cargo.toml");

    let output = Command::new(&cargo)
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--bin")
        .arg("cortex-claude-archive")
        .arg("--quiet")
        .env("CARGO_TARGET_DIR", tmp.path())
        .env("CARGO_INCREMENTAL", "0")
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("skipping feature_gates_it: cargo not invokable: {e}");
            return;
        }
    };

    assert!(
        !output.status.success(),
        "cargo build cortex-claude-archive without --features claude-archive should FAIL"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires the features") && stderr.contains("claude-archive"),
        "expected required-features error message; got: {stderr}"
    );
}

#[test]
fn cortex_claude_archive_bin_builds_with_feature() {
    if skip_unless_gated() {
        return;
    }
    let cargo = match std::env::var("CARGO") {
        Ok(v) => v,
        Err(_) => "cargo".to_string(),
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Cargo.toml");

    let output = Command::new(&cargo)
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--bin")
        .arg("cortex-claude-archive")
        .arg("--features")
        .arg("claude-archive")
        .arg("--quiet")
        .env("CARGO_TARGET_DIR", tmp.path())
        .env("CARGO_INCREMENTAL", "0")
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            eprintln!("skipping feature_gates_it (with-feature): cargo not invokable: {e}");
            return;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("expected build to succeed with --features claude-archive; stderr: {stderr}");
    }
}
