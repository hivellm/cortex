use std::process::ExitCode;

/// Phase9e — `cortex-ops turn-digest`. Today's surface is a
/// synthetic preview against the in-memory backend; the production
/// pipeline (Parquet walker → classifier → embedder → Nexus →
/// Parquet rewriter) lands with phase9k's cron scheduler. The CLI
/// prints the bucket plan + per-bucket outcomes so operators can
/// verify the spec contract before phase9k runs the live pipeline.
pub(super) fn turn_digest(
    time_travel: Option<String>,
    dry_run: bool,
    rebuild: bool,
    budget_cents: u64,
    json: bool,
) -> ExitCode {
    use cortex_workers::retention::turn_digest::{run_turn_digest, DigestPlan, MemoryDigestBackend, Turn};

    // phase11v §6 — bookkeeping anchor.
    let started_at = chrono::Utc::now();

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

    let mut plan = DigestPlan::default_for(now);
    plan.dry_run = dry_run;
    plan.rebuild = rebuild;
    plan.max_usd_cents_per_run = budget_cents;

    // Synthetic preview suite — 8 turns @ 60 days under (alpha,
    // ISO_week, "auth") plus 8 turns under (alpha, same week,
    // "ingestion"). Bucketize emits 2 buckets ≥ min_bucket_size=5.
    let mut turns = Vec::new();
    for topic in ["auth", "ingestion"] {
        for i in 0..8 {
            turns.push(Turn {
                event_id: format!("01PREVIEW-{topic}-{i}"),
                repo: "alpha".to_string(),
                occurred_at: now - chrono::Duration::days(60),
                top_topic: topic.to_string(),
                summarized_by: None,
            });
        }
    }

    let backend = MemoryDigestBackend::new();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let report = match runtime.block_on(run_turn_digest(&plan, &backend, turns)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("turn-digest: {e}");
            return ExitCode::FAILURE;
        }
    };

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops turn-digest (preview)");
        println!("now:                  {}", now.to_rfc3339());
        println!("dry_run:              {dry_run}");
        println!("rebuild:              {rebuild}");
        println!("budget_cents:         {budget_cents}");
        println!("examined:             {}", report.examined);
        println!("buckets_done:         {}", report.buckets_done);
        println!("already_digested:     {}", report.already_digested);
        println!("buckets_pending:      {}", report.buckets_pending);
        println!("usd_cents:            {}", report.usd_cents);
        for o in &report.outcomes {
            let label = if o.digested {
                "OK      "
            } else if o.already_digested {
                "ALREADY "
            } else if o.error.is_some() {
                "FAILED  "
            } else {
                "PENDING "
            };
            println!("  {label}  {}", o.key);
        }
    }
    let mut extras = serde_json::Map::new();
    extras.insert("examined".into(), report.examined.into());
    extras.insert("buckets_done".into(), report.buckets_done.into());
    extras.insert("already_digested".into(), report.already_digested.into());
    extras.insert("buckets_pending".into(), report.buckets_pending.into());
    extras.insert("usd_cents".into(), report.usd_cents.into());
    super::record_sweep_run(
        "turn_digest",
        started_at,
        "success",
        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
            extras,
            ..Default::default()
        },
    );
    ExitCode::SUCCESS
}

/// phase11w — `cortex-ops tool-call-digest`. Synthetic preview of
/// the bucketise + summarise + (optional) hard-purge pipeline for
/// `tool_call` envelopes. Today drives the in-memory backend so
/// operators can verify the `(repo, year_week, tool)` shape before
/// the live Sonnet + Meili `delete-batch` + Vectorizer
/// `delete_vectors` + Parquet rewriter wiring lands. Default mode
/// is preview (no classifier call, no deletes); `--purge-originals`
/// (paired with the implicit non-dry-run) flips the deletes ON.
pub(super) fn tool_call_digest(
    time_travel: Option<String>,
    dry_run: bool,
    rebuild: bool,
    budget_cents: u64,
    purge_originals: bool,
    json: bool,
) -> ExitCode {
    use cortex_workers::retention::tool_call_digest::{
        run_tool_call_digest, DigestPlan, MemoryToolCallDigestBackend, ToolCall,
    };

    let started_at = chrono::Utc::now();

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

    let mut plan = DigestPlan::default_for(now);
    plan.dry_run = dry_run;
    plan.rebuild = rebuild;
    plan.max_usd_cents_per_run = budget_cents;
    plan.purge_originals = purge_originals;

    // Synthetic preview suite: 6 Bash + 5 Read at 60 d old in repo
    // `alpha` → 2 buckets above `min_bucket_size=5`. 4 Edit calls
    // (below threshold) verify the drop path.
    let mut tool_calls = Vec::new();
    for tool in ["Bash", "Read"] {
        let count = if tool == "Bash" { 6 } else { 5 };
        for i in 0..count {
            tool_calls.push(ToolCall {
                event_id: format!("01PREVIEW-{tool}-{i}"),
                repo: "alpha".to_string(),
                occurred_at: now - chrono::Duration::days(60),
                tool: tool.to_string(),
                summarized_by: None,
            });
        }
    }
    for i in 0..4 {
        tool_calls.push(ToolCall {
            event_id: format!("01PREVIEW-Edit-{i}"),
            repo: "alpha".to_string(),
            occurred_at: now - chrono::Duration::days(60),
            tool: "Edit".to_string(),
            summarized_by: None,
        });
    }

    let backend = MemoryToolCallDigestBackend::new();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let report = match runtime.block_on(run_tool_call_digest(&plan, &backend, tool_calls)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("tool-call-digest: {e}");
            super::record_sweep_run(
                "tool_call_digest",
                started_at,
                "failed",
                cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
                    last_error: Some(format!("{e}").chars().take(256).collect()),
                    ..Default::default()
                },
            );
            return ExitCode::FAILURE;
        }
    };

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops tool-call-digest (preview)");
        println!("now:                  {}", now.to_rfc3339());
        println!("dry_run:              {dry_run}");
        println!("rebuild:              {rebuild}");
        println!("purge_originals:      {purge_originals}");
        println!("budget_cents:         {budget_cents}");
        println!("examined:             {}", report.examined);
        println!("buckets_done:         {}", report.buckets_done);
        println!("already_digested:     {}", report.already_digested);
        println!("buckets_pending:      {}", report.buckets_pending);
        println!("records_purged:       {}", report.records_purged);
        println!("usd_cents:            {}", report.usd_cents);
        for o in &report.outcomes {
            let label = if o.digested {
                "OK      "
            } else if o.already_digested {
                "ALREADY "
            } else if o.error.is_some() {
                "ERROR   "
            } else {
                "PENDING "
            };
            println!("  {label}  {}  purged={}", o.key, o.purged);
        }
    }
    let mut extras = serde_json::Map::new();
    extras.insert("examined".into(), report.examined.into());
    extras.insert("buckets_done".into(), report.buckets_done.into());
    extras.insert("already_digested".into(), report.already_digested.into());
    extras.insert("buckets_pending".into(), report.buckets_pending.into());
    extras.insert("records_purged".into(), report.records_purged.into());
    extras.insert("usd_cents".into(), report.usd_cents.into());
    extras.insert("purge_originals".into(), purge_originals.into());
    super::record_sweep_run(
        "tool_call_digest",
        started_at,
        "success",
        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
            records_dropped: report.records_purged,
            extras,
            ..Default::default()
        },
    );
    ExitCode::SUCCESS
}
