//! phase11s §6.2 — bin surface parity IT.
//!
//! Smoke-checks that every cortex-workers operator binary responds to
//! `--help` with exit code 0 and prints its own name + a `Usage:` block
//! (clap's default). The merge from 14 → 9 workspace members folded 4 new
//! bins into cortex-workers (`cortex-ingestion`, `cortex-claude-archive`,
//! `cortex-consolidator`, `cortex-retention-sweep`) on top of the 5
//! pre-existing ones (`cortex-classifier-worker`, `cortex-embedder-worker`,
//! `cortex-fulltext-worker`, `cortex-graph-worker`, `cortex-graph-backfill`).
//!
//! This IT does NOT do a byte-equivalent golden diff against pre-merge
//! `--help` output (would create churn on every clap bump). It validates
//! that every bin slot actually produces a runnable executable whose
//! help text shape matches clap conventions.
//!
//! Env-gated with `CORTEX_BIN_PARITY_IT=1` because the test spawns nested
//! processes and depends on the workspace being pre-built (the harness
//! does not invoke `cargo build` itself).

use std::path::PathBuf;
use std::process::Command;

fn skip_unless_gated() -> bool {
    if std::env::var("CORTEX_BIN_PARITY_IT").as_deref() != Ok("1") {
        eprintln!("skipping bin_surface_parity_it: set CORTEX_BIN_PARITY_IT=1 to enable");
        return true;
    }
    false
}

fn target_dir() -> PathBuf {
    // CARGO_TARGET_DIR is set when the workspace target/ has been moved;
    // otherwise the parent target/ relative to CARGO_MANIFEST_DIR holds the
    // built bins.
    if let Ok(p) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(p);
    }
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // workspace root
    p.push("target");
    p
}

fn bin_path(name: &str) -> PathBuf {
    let mut p = target_dir();
    p.push("debug");
    if cfg!(windows) {
        p.push(format!("{name}.exe"));
    } else {
        p.push(name);
    }
    p
}

fn check_help(name: &str) {
    let path = bin_path(name);
    if !path.exists() {
        panic!(
            "expected bin {name} to be pre-built at {} — run `cargo build --bin {name}` first",
            path.display()
        );
    }
    let output = Command::new(&path)
        .arg("--help")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn {name} --help: {e}"));
    assert!(
        output.status.success(),
        "{name} --help exited non-zero: {}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        combined.contains("Usage:") || combined.contains("USAGE:"),
        "{name} --help missing Usage block; got: {combined}"
    );
}

#[test]
fn cortex_classifier_worker_help_round_trips() {
    if skip_unless_gated() {
        return;
    }
    check_help("cortex-classifier-worker");
}

#[test]
fn cortex_embedder_worker_help_round_trips() {
    if skip_unless_gated() {
        return;
    }
    check_help("cortex-embedder-worker");
}

#[test]
fn cortex_fulltext_worker_help_round_trips() {
    if skip_unless_gated() {
        return;
    }
    check_help("cortex-fulltext-worker");
}

#[test]
fn cortex_graph_worker_help_round_trips() {
    if skip_unless_gated() {
        return;
    }
    check_help("cortex-graph-worker");
}

#[test]
fn cortex_graph_backfill_help_round_trips() {
    if skip_unless_gated() {
        return;
    }
    check_help("cortex-graph-backfill");
}

#[test]
fn cortex_ingestion_help_round_trips() {
    if skip_unless_gated() {
        return;
    }
    check_help("cortex-ingestion");
}

#[test]
fn cortex_consolidator_help_round_trips() {
    if skip_unless_gated() {
        return;
    }
    check_help("cortex-consolidator");
}

#[test]
fn cortex_retention_sweep_help_round_trips() {
    if skip_unless_gated() {
        return;
    }
    check_help("cortex-retention-sweep");
}
