//! `cortex-retention-sweep` — phase11s §5.4 operator binary for the
//! Vectorizer tier-transition sweep.
//!
//! Today the canonical operator surface for the retention sweep is
//! `cortex-ops sweep` (lives in `cortex-cli/src/bin/cortex-ops.rs`).
//! That subcommand calls `cortex_workers::retention::run_sweep` against
//! the live Vectorizer SDK + the metadata store. This binary exposes
//! the same library entry point under the cron-friendly name
//! `cortex-retention-sweep` so process supervisors that key off bin
//! names (systemd timers, docker-compose entry points) have a stable
//! handle without mounting the full `cortex-ops` CLI.
//!
//! Today it ships with a deliberately narrow surface: `--dry-run`
//! (default `true` so a stray invocation never mutates production
//! state) and `--time-travel <RFC3339>` (override the reference time
//! for back-dated sweeps + tests). The live Vectorizer adapter +
//! metadata-store wiring lives in `cortex-cli/src/bin/cortex-ops.rs`
//! and is not duplicated here — instead the bin exits with a clear
//! pointer to `cortex-ops sweep` for live runs while the in-process
//! adapter is in flight (a follow-up phase will lift the
//! `LiveVectorizerOps` adapter from `cortex-ops` into
//! `cortex_workers::retention::live` so this bin can call it
//! directly).
//!
//! Until then the value of this bin is twofold:
//!
//! 1. The dry-run path validates plan construction + cutoff math
//!    against the in-memory `MemoryVectorizerOps`, which is what the
//!    canary IT (`tests/retention_canary.rs`) exercises. Operators can
//!    invoke it offline to confirm the default plan looks sane.
//! 2. The bin name slot is reserved so the docker-compose / systemd
//!    config does not have to chase a renamed entry point when the
//!    live adapter lands.

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use cortex_workers::retention::{run_sweep, MemoryVectorizerOps, SweepPlan};

#[derive(Debug, Parser)]
#[command(
    name = "cortex-retention-sweep",
    about = "Vectorizer tier-transition sweep (phase11s §5.4 operator entry point).",
    version
)]
struct Cli {
    /// Reference time. RFC3339 (e.g. `2026-04-29T12:00:00Z`). Defaults
    /// to wall-clock `Utc::now()` when omitted.
    #[arg(long)]
    time_travel: Option<String>,
    /// `true` (default) skips every Vectorizer mutation. Pass
    /// `--no-dry-run` when running from the operator surface to opt
    /// in. Live mutations require the cortex-ops in-process adapter
    /// path; this binary deliberately stays read-only until that
    /// adapter is lifted into the cortex_workers::retention module.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    dry_run: bool,
    /// Verbose tracing output.
    #[arg(long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    let now = match cli.time_travel.as_deref() {
        Some(s) => DateTime::parse_from_rfc3339(s)
            .with_context(|| format!("--time-travel: not RFC3339: {s}"))?
            .with_timezone(&Utc),
        None => Utc::now(),
    };
    let mut plan = SweepPlan::default_for(now);
    plan.dry_run = cli.dry_run;
    if !cli.dry_run {
        return Err(anyhow::anyhow!(
            "live tier-transition sweep is hosted by `cortex-ops sweep` \
             (lives in cortex-cli) until the LiveVectorizerOps adapter \
             is lifted into cortex_workers::retention; rerun with \
             `--dry-run=true` for an offline plan validation"
        ));
    }
    let ops = MemoryVectorizerOps::new();
    let report = run_sweep(&plan, &ops).await?;
    println!("cortex-retention-sweep --dry-run plan");
    println!("  now            : {now}");
    println!("  pairs          : {}", plan.pairs.len());
    println!("  batch_size     : {}", plan.batch_size);
    println!("  max_error_rate : {}", plan.max_error_rate);
    println!("  records_demoted: {}", report.records_demoted);
    println!("  records_dropped: {}", report.records_dropped);
    println!("  transitions    : {}", report.transitions.len());
    Ok(())
}

fn init_tracing(verbose: bool) {
    let level = if verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .try_init();
}
