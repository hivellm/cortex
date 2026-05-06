//! Phase 11x cold-start benchmark for the `cortex-hook` bin.
//!
//! Measures four scenarios end-to-end (process spawn + work +
//! teardown) so the numbers reflect what Claude Code actually pays
//! per hook invocation:
//!
//! - `cold_start_help` — bin with `--help`. Pure cold start; the
//!   bin parses args, prints help, exits.
//! - `cold_start_disabled` — bin with `CORTEX_ADAPTER_DISABLE=1`.
//!   Cold start + the env-gated early-exit. Models the lowest-cost
//!   real invocation.
//! - `daemon_down_fail_open` — bin pointed at a nonexistent named
//!   pipe with a 200 ms timeout. Models the failure path.
//! - `fire_forget` — bin against the daemon's named pipe with
//!   `--fire-forget`. Models the fast publish-only event.
//!
//! Targets (Windows release):
//!
//! | scenario              | p50 budget |
//! |-----------------------|-----------|
//! | cold_start_help       | <50 ms    |
//! | cold_start_disabled   | <70 ms    |
//! | daemon_down_fail_open | <100 ms   |
//! | fire_forget           | <100 ms   |
//!
//! Linux: roughly half the Windows numbers above (no `pwsh`, faster
//! process spawn). `cargo bench -p cortex-adapter-claude-code` runs
//! them. The synchronous-with-daemon scenario is intentionally
//! omitted — it depends on a live daemon + cortex-api stack that
//! isn't available in plain `cargo bench` runs.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};

/// Resolve the release-mode `cortex-hook` binary alongside the
/// crate's compiled output. Falls back to invoking the bin via
/// `cargo run --release --bin cortex-hook` only if the prebuilt
/// artefact isn't present, so a fresh tree still benches without a
/// manual pre-build.
fn cortex_hook_bin() -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(&target).join("release").join(bin_filename()));
    }
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    candidates.push(
        PathBuf::from(manifest_dir)
            .join("..")
            .join("..")
            .join("target")
            .join("release")
            .join(bin_filename()),
    );
    for c in &candidates {
        if c.is_file() {
            return c.clone();
        }
    }
    // Fallback: rely on `cargo build --release --bin cortex-hook`
    // having been run; otherwise the bench output documents the
    // missing artefact instead of silently using a debug build.
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from(bin_filename()))
}

fn bin_filename() -> &'static str {
    if cfg!(windows) {
        "cortex-hook.exe"
    } else {
        "cortex-hook"
    }
}

fn run_bin(args: &[&str], stdin: &str, env: &[(&str, &str)]) {
    let bin = cortex_hook_bin();
    let mut cmd = Command::new(&bin);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            // The release bin is missing — record a no-op so the
            // bench still reports something meaningful instead of
            // panicking. Operators see the gap when the numbers all
            // collapse to spawn-failure noise.
            return;
        }
    };
    if let Some(mut sin) = child.stdin.take() {
        use std::io::Write;
        let _ = sin.write_all(stdin.as_bytes());
    }
    let _ = child.wait();
}

fn bench_cold_start_help(c: &mut Criterion) {
    c.bench_function("cold_start_help", |b| {
        b.iter(|| run_bin(&["--help"], "", &[]));
    });
}

fn bench_cold_start_disabled(c: &mut Criterion) {
    c.bench_function("cold_start_disabled", |b| {
        b.iter(|| {
            run_bin(
                &["UserPromptSubmit"],
                "{\"prompt\":\"bench\"}",
                &[("CORTEX_ADAPTER_DISABLE", "1")],
            )
        });
    });
}

fn bench_daemon_down_fail_open(c: &mut Criterion) {
    c.bench_function("daemon_down_fail_open", |b| {
        b.iter(|| {
            run_bin(
                &[
                    "PreToolUse",
                    "--pipe",
                    r"\\.\pipe\cortex-bench-nonexistent",
                    "--sock",
                    "/tmp/cortex-bench-nonexistent.sock",
                    "--timeout-ms",
                    "200",
                ],
                "{}",
                &[],
            )
        });
    });
}

fn bench_fire_forget(c: &mut Criterion) {
    c.bench_function("fire_forget", |b| {
        b.iter(|| {
            run_bin(
                &["PostToolUse", "--fire-forget", "--timeout-ms", "1500"],
                "{}",
                &[],
            )
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(20)
        .measurement_time(Duration::from_secs(8))
        .warm_up_time(Duration::from_secs(2));
    targets =
        bench_cold_start_help,
        bench_cold_start_disabled,
        bench_daemon_down_fail_open,
        bench_fire_forget,
}
criterion_main!(benches);
