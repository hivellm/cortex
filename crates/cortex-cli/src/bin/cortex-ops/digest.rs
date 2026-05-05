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

/// phase11w — `cortex-ops tool-call-digest`. Two execution modes:
///
/// 1. **Synthetic preview** (default, `--apply` off) — drives an
///    in-process backend with a deterministic suite (6 Bash + 5 Read
///    + 4 Edit, all 60 d old). Lets operators verify the
///    `(repo, year_week, tool)` bucketisation contract without
///    touching live state.
/// 2. **Live** (`--apply` on) — paginates Meili's `cortex_tool_calls`
///    index for everything older than the cutoff, runs each bucket
///    through the live `cortex-ingestion` + (when paired with
///    `--purge-originals`) `cortex-api /v1/admin/forget` cascade.
///    The cron schedule (`retention.tool_call_digest`) ships this
///    flag set.
pub(super) fn tool_call_digest(
    time_travel: Option<String>,
    dry_run: bool,
    rebuild: bool,
    budget_cents: u64,
    apply: bool,
    purge_originals: bool,
    max_records: usize,
    page_size: u32,
    json: bool,
) -> ExitCode {
    use cortex_workers::retention::tool_call_digest::{
        run_tool_call_digest, DigestPlan, MemoryToolCallDigestBackend, ToolCall,
        ToolCallDigestBackend,
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

    // Build the source list + backend either as the synthetic
    // preview pair or as the live Meili → cortex-api pair.
    let (tool_calls, backend, mode_label): (Vec<ToolCall>, Box<dyn ToolCallDigestBackend>, &str) =
        if apply {
            let api_base = std::env::var("CORTEX_API_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:17000".to_string());
            let api_token = std::env::var("CORTEX_API_TOKEN").ok();
            let ingestion_base = std::env::var("CORTEX_INGESTION_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:17010".to_string());
            let meili_base = std::env::var("CORTEX_FULLTEXT_MEILI_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:7700".to_string());
            let meili_key = std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY")
                .ok()
                .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_KEY").ok())
                .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());
            let cutoff = now - chrono::Duration::days(plan.digest_after_days);
            let live = match super::tool_call_digest_live::LiveToolCallDigestBackend::new(
                api_base,
                api_token,
                ingestion_base,
                meili_base.clone(),
                meili_key.clone(),
            ) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("tool-call-digest: live backend init: {e:#}");
                    return ExitCode::FAILURE;
                }
            };
            let fetched = match runtime.block_on(super::tool_call_digest_live::fetch_old_tool_calls(
                &meili_base,
                meili_key.as_deref(),
                cutoff,
                page_size,
                max_records,
            )) {
                Ok(rows) => rows,
                Err(e) => {
                    eprintln!("tool-call-digest: meili enumerator: {e:#}");
                    super::record_sweep_run(
                        "tool_call_digest",
                        started_at,
                        "failed",
                        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
                            last_error: Some(
                                format!("meili enumerator: {e}").chars().take(256).collect(),
                            ),
                            ..Default::default()
                        },
                    );
                    return ExitCode::FAILURE;
                }
            };
            (fetched, Box::new(live), "live")
        } else {
            let mut synthetic = Vec::new();
            for tool in ["Bash", "Read"] {
                let count = if tool == "Bash" { 6 } else { 5 };
                for i in 0..count {
                    synthetic.push(ToolCall {
                        event_id: format!("01PREVIEW-{tool}-{i}"),
                        repo: "alpha".to_string(),
                        occurred_at: now - chrono::Duration::days(60),
                        tool: tool.to_string(),
                        summarized_by: None,
                    });
                }
            }
            for i in 0..4 {
                synthetic.push(ToolCall {
                    event_id: format!("01PREVIEW-Edit-{i}"),
                    repo: "alpha".to_string(),
                    occurred_at: now - chrono::Duration::days(60),
                    tool: "Edit".to_string(),
                    summarized_by: None,
                });
            }
            (synthetic, Box::new(MemoryToolCallDigestBackend::new()), "preview")
        };

    let fetched_count = tool_calls.len();
    let report = match runtime.block_on(run_tool_call_digest(&plan, backend.as_ref(), tool_calls)) {
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
        println!("cortex-ops tool-call-digest ({mode_label})");
        println!("now:                  {}", now.to_rfc3339());
        println!("dry_run:              {dry_run}");
        println!("rebuild:              {rebuild}");
        println!("purge_originals:      {purge_originals}");
        println!("budget_cents:         {budget_cents}");
        println!("source_rows:          {fetched_count}");
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
            if let Some(err) = &o.error {
                println!("    error: {err}");
            }
        }
    }
    let mut extras = serde_json::Map::new();
    extras.insert("mode".into(), mode_label.into());
    extras.insert("source_rows".into(), fetched_count.into());
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
