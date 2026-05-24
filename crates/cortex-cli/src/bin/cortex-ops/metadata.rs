use super::{
    helpers::{home_dir, resolve_metadata_db},
    LogConfigOverrides, MetadataReapTargetArg,
};
use std::process::ExitCode;

/// Phase9g — `cortex-ops metadata-reap`. Aggregates aged SQLite rows
/// into rollup tables, deletes the sources, and rotates the operator
/// hook logs. Bookkeeping rides on the same `retention_sweeps` row
/// machinery as phase9a so the dashboard sees one entry per run.
pub(super) fn metadata_reap(
    time_travel: Option<String>,
    dry_run: bool,
    target: MetadataReapTargetArg,
    metadata_db: Option<String>,
    log_dir: Option<String>,
    skip_logs: bool,
    json: bool,
) -> ExitCode {
    use cortex_cli::ops::{rotate_if_needed, LogRotateOpts, LogRotateOutcome};
    use cortex_storage::MetadataStore;
    use cortex_workers::retention::metadata_reap::{run, ReapPlan, ReapTarget};

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let metadata_path = metadata_db
        .map(std::path::PathBuf::from)
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.ingestion.metadata_db)
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex").join("metadata.sqlite"))
                .unwrap_or_else(|| std::path::PathBuf::from(".cortex/metadata.sqlite"))
        });

    let log_dir = log_dir.map(std::path::PathBuf::from).unwrap_or_else(|| {
        home_dir()
            .map(|h| h.join(".cortex"))
            .unwrap_or_else(|| std::path::PathBuf::from(".cortex"))
    });

    // SQLite rollup. The `Logs` target skips this branch entirely.
    let only_logs = matches!(target, MetadataReapTargetArg::Logs);
    let mut store = if only_logs {
        None
    } else {
        match MetadataStore::open(&metadata_path) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("metadata open ({}): {e}", metadata_path.display());
                return ExitCode::FAILURE;
            }
        }
    };

    let sweep_id = cortex_workers::retention::new_sweep_id();
    if let Some(s) = store.as_ref() {
        if let Err(e) = s.start_retention_sweep(&sweep_id, now, 3600) {
            eprintln!("metadata-reap: {e}");
            // Code 2 — another sweep in flight (per spec).
            return ExitCode::from(2);
        }
    }

    let mut plan = ReapPlan::default_for(now);
    plan.dry_run = dry_run;
    plan.target = match target {
        MetadataReapTargetArg::All => ReapTarget::All,
        MetadataReapTargetArg::BootstrapJobs => ReapTarget::BootstrapJobs,
        MetadataReapTargetArg::Sessions => ReapTarget::Sessions,
        MetadataReapTargetArg::ClassifierSpend => ReapTarget::ClassifierSpend,
        MetadataReapTargetArg::Logs => ReapTarget::All, // unused (only_logs branch above)
    };

    // Apply `cortex.toml [retention.metadata]` overrides when the
    // file is present in `<home>/.cortex/cortex.toml`. Missing file,
    // missing section, missing keys all silently fall back to the
    // spec defaults.
    let cortex_toml = home_dir()
        .map(|h| h.join(".cortex").join("cortex.toml"))
        .filter(|p| p.exists());
    let mut log_overrides = LogConfigOverrides::default();
    if let Some(p) = cortex_toml.as_ref() {
        match std::fs::read_to_string(p) {
            Ok(body) => match body.parse::<toml::Value>() {
                Ok(root) => {
                    if let Some(section) = root
                        .get("retention")
                        .and_then(|v| v.get("metadata"))
                        .and_then(|v| v.as_table())
                    {
                        if let Some(v) = section
                            .get("bootstrap_retain_days")
                            .and_then(|v| v.as_integer())
                        {
                            plan.bootstrap_retain_days = v;
                        }
                        if let Some(v) = section
                            .get("sessions_retain_days")
                            .and_then(|v| v.as_integer())
                        {
                            plan.sessions_retain_days = v;
                        }
                        if let Some(v) = section
                            .get("spend_retain_days")
                            .and_then(|v| v.as_integer())
                        {
                            plan.spend_retain_days = v;
                        }
                        if let Some(v) = section.get("log_max_bytes").and_then(|v| v.as_integer()) {
                            log_overrides.max_bytes = u64::try_from(v).ok();
                        }
                        if let Some(v) =
                            section.get("log_max_age_days").and_then(|v| v.as_integer())
                        {
                            log_overrides.max_age_days = u32::try_from(v).ok();
                        }
                        if let Some(v) = section
                            .get("log_keep_rotations")
                            .and_then(|v| v.as_integer())
                        {
                            log_overrides.keep_rotations = usize::try_from(v).ok();
                        }
                    }
                }
                Err(e) => tracing::warn!(error=%e, "cortex.toml parse failed"),
            },
            Err(e) => tracing::warn!(error=%e, "cortex.toml read failed"),
        }
    }

    let report = if let Some(s) = store.as_mut() {
        match run(s.conn_mut(), &plan) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("metadata-reap: {e}");
                let _ =
                    s.finish_retention_sweep(&sweep_id, chrono::Utc::now(), 0, 0, "{}", "failed");
                return ExitCode::FAILURE;
            }
        }
    } else {
        cortex_workers::retention::metadata_reap::ReapReport::default()
    };

    // Log rotation. Always runs unless `--skip-logs`. The reaper
    // tolerates failures here (operator may have a read-only mount)
    // — they surface in the report payload but never fail the run.
    let mut log_outcomes: Vec<(std::path::PathBuf, LogRotateOutcome)> = Vec::new();
    if !skip_logs && !dry_run {
        let mut log_opts = LogRotateOpts::default_for(now);
        if let Some(v) = log_overrides.max_bytes {
            log_opts.max_bytes = v;
        }
        if let Some(v) = log_overrides.max_age_days {
            log_opts.max_age_days = v;
        }
        if let Some(v) = log_overrides.keep_rotations {
            log_opts.keep_rotations = v;
        }
        for name in ["hook-invocations.log", "hook-errors.log"] {
            let p = log_dir.join(name);
            match rotate_if_needed(&p, &log_opts) {
                Ok(o) => log_outcomes.push((p, o)),
                Err(e) => {
                    tracing::warn!(error=%e, path=%p.display(), "log rotation failed");
                }
            }
        }
    }

    // Bookkeeping. `records_demoted` carries the SUM of source rows
    // collapsed across all targets so the dashboard's existing
    // ribbon stays meaningful; `tier_transitions_json` carries the
    // per-target breakdown plus log-rotation counters.
    let collapsed_total =
        report.bootstrap_jobs_collapsed + report.sessions_collapsed + report.spend_collapsed;
    let bookkeeping_json = bookkeeping_payload(&report, &log_outcomes);
    let finished_at = chrono::Utc::now();
    if let Some(s) = store.as_ref() {
        let status = "success";
        if let Err(e) = s.finish_retention_sweep(
            &sweep_id,
            finished_at,
            collapsed_total,
            0,
            &bookkeeping_json,
            status,
        ) {
            eprintln!("metadata-reap: bookkeeping write failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    if json {
        let payload = serde_json::json!({
            "sweep_id": sweep_id,
            "started_at": now.to_rfc3339(),
            "finished_at": finished_at.to_rfc3339(),
            "dry_run": dry_run,
            "target": format!("{:?}", target).to_lowercase(),
            "metadata_reap": serde_json::from_str::<serde_json::Value>(
                &report.metadata_reap_json(),
            )
            .unwrap_or_else(|_| serde_json::json!({})),
            "log_rotations": log_outcomes
                .iter()
                .map(|(p, o)| {
                    serde_json::json!({
                        "path": p.display().to_string(),
                        "rotated": o.rotated,
                        "source_bytes": o.source_bytes,
                        "gz_path": o.gz_path.as_ref().map(|p| p.display().to_string()),
                        "pruned": o.pruned.iter().map(|p| p.display().to_string())
                            .collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops metadata-reap");
        println!("sweep_id:        {sweep_id}");
        println!("now:             {}", now.to_rfc3339());
        println!("dry_run:         {dry_run}");
        println!("metadata_db:     {}", metadata_path.display());
        println!(
            "bootstrap_jobs:  collapsed={} buckets={}",
            report.bootstrap_jobs_collapsed, report.bootstrap_daily_buckets
        );
        println!(
            "sessions:        collapsed={} buckets={}",
            report.sessions_collapsed, report.sessions_monthly_buckets
        );
        println!(
            "classifier_spend:collapsed={} buckets={}",
            report.spend_collapsed, report.spend_monthly_buckets
        );
        println!(
            "free_pages:      ratio={:.2} did_vacuum={} vacuum_ms={}",
            report.free_pages_ratio, report.did_vacuum, report.vacuum_ms
        );
        if log_outcomes.is_empty() {
            println!("logs:            (skipped)");
        } else {
            println!("logs:");
            for (p, o) in &log_outcomes {
                if o.rotated {
                    println!(
                        "  rotated {} ({} B) → {}",
                        p.display(),
                        o.source_bytes,
                        o.gz_path
                            .as_ref()
                            .map(|q| q.display().to_string())
                            .unwrap_or_default()
                    );
                    if !o.pruned.is_empty() {
                        for old in &o.pruned {
                            println!("    pruned {}", old.display());
                        }
                    }
                } else {
                    println!("  skipped {} ({} B)", p.display(), o.source_bytes);
                }
            }
        }
    }
    ExitCode::SUCCESS
}

fn bookkeeping_payload(
    report: &cortex_workers::retention::metadata_reap::ReapReport,
    log_outcomes: &[(std::path::PathBuf, cortex_cli::ops::LogRotateOutcome)],
) -> String {
    let metadata_reap = serde_json::from_str::<serde_json::Value>(&report.metadata_reap_json())
        .unwrap_or_else(|_| serde_json::json!({}));
    let logs: Vec<serde_json::Value> = log_outcomes
        .iter()
        .map(|(p, o)| {
            serde_json::json!({
                "path": p.display().to_string(),
                "rotated": o.rotated,
                "source_bytes": o.source_bytes,
            })
        })
        .collect();
    serde_json::json!({
        "metadata_reap": metadata_reap,
        "log_rotations": logs,
    })
    .to_string()
}

/// Phase10i — `cortex-ops sessions backfill-tool` dispatcher.
/// Walks the metadata `sessions` table for rows whose `tool` is
/// NULL or empty and stamps `--tool` on each. `--dry-run` lists
/// candidate ids without mutating; `--apply` runs the update.
pub(super) fn sessions_backfill_tool(
    tool: String,
    dry_run: bool,
    apply: bool,
    limit: usize,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    if !dry_run && !apply {
        eprintln!("sessions backfill-tool: pass either --dry-run or --apply");
        return ExitCode::from(2);
    }
    if tool.trim().is_empty() {
        eprintln!("sessions backfill-tool: --tool must not be empty");
        return ExitCode::from(2);
    }
    let db_path = match resolve_metadata_db(metadata_db) {
        Some(p) => p,
        None => {
            eprintln!("sessions backfill-tool: cannot resolve metadata DB path");
            return ExitCode::from(2);
        }
    };
    let store = match cortex_storage::MetadataStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sessions backfill-tool: open {}: {e}", db_path.display());
            return ExitCode::from(1);
        }
    };
    let candidates = match store.sessions_missing_tool(limit) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sessions backfill-tool: scan: {e}");
            return ExitCode::from(1);
        }
    };
    let candidate_count = candidates.len() as u64;
    let mut stamped: u64 = 0;
    if apply {
        for (sid, _started) in &candidates {
            match store.set_session_tool(sid, &tool) {
                Ok(touched) => stamped += touched,
                Err(e) => {
                    eprintln!("sessions backfill-tool: stamp {sid}: {e}");
                    return ExitCode::from(1);
                }
            }
        }
    }
    if json {
        let payload = serde_json::json!({
            "mode": if apply { "apply" } else { "dry-run" },
            "tool": tool,
            "candidates": candidate_count,
            "stamped": stamped,
            "limit": limit,
            "preview": candidates
                .iter()
                .take(10)
                .map(|(sid, started)| serde_json::json!({
                    "session_id": sid,
                    "started_at": started,
                }))
                .collect::<Vec<_>>(),
        });
        println!("{payload}");
    } else {
        println!(
            "sessions backfill-tool ({} mode):",
            if apply { "apply" } else { "dry-run" }
        );
        println!("  tool to stamp:           {tool}");
        println!("  candidate rows:          {candidate_count}");
        println!("  rows stamped:            {stamped}");
        println!("  limit (per invocation):  {limit}");
        let preview = candidates.iter().take(5);
        for (sid, started) in preview {
            println!("    {sid}  ({started})");
        }
        if candidate_count as usize > 5 {
            println!("    ... ({} more)", candidate_count.saturating_sub(5));
        }
    }
    ExitCode::SUCCESS
}
