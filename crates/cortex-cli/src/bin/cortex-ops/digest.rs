use std::process::ExitCode;

/// Phase9e + phase11x — `cortex-ops turn-digest`. Two execution modes:
///
/// 1. **Synthetic preview** (default, `--apply` off) — 8 + 8 turns
///    across two topics in an in-memory backend.
/// 2. **Live** (`--apply` on) — paginates every per-repo
///    `cortex-<repo>-turns` Meili index filtered by `kind=turn`,
///    runs each bucket through `cortex-ingestion` + (when paired
///    with `--purge-originals`) `cortex-api /v1/admin/forget`.
#[allow(clippy::too_many_arguments)]
pub(super) fn turn_digest(
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
    use cortex_workers::retention::turn_digest::{
        run_turn_digest, DigestBackend, DigestPlan, MemoryDigestBackend, Turn,
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

    let cfg = cortex_config::Config::load().unwrap_or_default();
    let api_base = cfg
        .dashboard
        .api_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:17000".to_string());
    let api_token = cfg.dashboard.api_token.clone();
    let ingestion_base = cfg
        .ingestion
        .ingestion_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:17010".to_string());
    let meili_base = cfg
        .meili
        .meili_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:7700".to_string());
    let meili_key = cfg
        .meili
        .meili_api_key
        .clone()
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let _ = page_size; // reserved for future Meili-direct fallback path
    let (turns, mode_label, live_for_purge): (
        Vec<Turn>,
        &str,
        Option<super::turn_digest_live::LiveTurnDigestBackend>,
    ) = if apply {
        let cutoff = now - chrono::Duration::days(plan.digest_after_days);
        let fetched = match runtime.block_on(super::turn_digest_live::fetch_old_turns_via_admin(
            &api_base,
            api_token.as_deref(),
            cutoff,
            max_records,
        )) {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("turn-digest: meili enumerator: {e:#}");
                super::record_sweep_run(
                    "turn_digest",
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
        let purger = if purge_originals && !dry_run {
            match super::turn_digest_live::LiveTurnDigestBackend::new(
                api_base.clone(),
                api_token.clone(),
                ingestion_base.clone(),
                meili_base.clone(),
                meili_key.clone(),
            ) {
                Ok(b) => Some(b),
                Err(e) => {
                    eprintln!("turn-digest: purge backend init: {e:#}");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            None
        };
        (fetched, "live", purger)
    } else {
        let mut synthetic = Vec::new();
        for topic in ["auth", "ingestion"] {
            for i in 0..8 {
                synthetic.push(Turn {
                    event_id: format!("01PREVIEW-{topic}-{i}"),
                    repo: "alpha".to_string(),
                    occurred_at: now - chrono::Duration::days(60),
                    top_topic: topic.to_string(),
                    summarized_by: None,
                });
            }
        }
        (synthetic, "preview", None)
    };

    let fetched_count = turns.len();
    // Capture per-bucket event ids before run_turn_digest consumes
    // the turns vec — the post-run purge cascade needs them.
    let mut bucket_event_ids: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    if live_for_purge.is_some() {
        for t in &turns {
            let key = format!(
                "{}|{}|{}",
                t.repo,
                cortex_workers::retention::turn_digest::iso_year_week(t.occurred_at),
                t.top_topic,
            );
            bucket_event_ids
                .entry(key)
                .or_default()
                .push(t.event_id.clone());
        }
    }

    let backend: Box<dyn DigestBackend> = if apply {
        match super::turn_digest_live::LiveTurnDigestBackend::new(
            api_base,
            api_token,
            ingestion_base,
            meili_base,
            meili_key,
        ) {
            Ok(b) => Box::new(b),
            Err(e) => {
                eprintln!("turn-digest: backend init: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        Box::new(MemoryDigestBackend::new())
    };

    let report = match runtime.block_on(run_turn_digest(&plan, backend.as_ref(), turns)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("turn-digest: {e}");
            super::record_sweep_run(
                "turn_digest",
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

    // SAFETY GATE — purge cascade is gated on each digest envelope
    // being queryable in Meili. The ingestion pipeline is async; the
    // POST in `persist_digest` returns before the classifier +
    // embedder + fulltext-worker chain finishes indexing. Calling
    // `/v1/admin/forget` on the source rows BEFORE the digest is
    // visible would orphan originals if any downstream stage drops
    // the envelope (classifier rejection, embedder error, Meili
    // task failure). Per user constraint 2026-05-05: data may only
    // be removed after the summarisation lands.
    let mut records_purged = 0u64;
    let mut buckets_purge_skipped: Vec<(String, &'static str)> = Vec::new();
    if let Some(live) = live_for_purge.as_ref() {
        const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
        for outcome in &report.outcomes {
            if !outcome.digested {
                continue;
            }
            // Recover the (repo, year_week, top_topic) triple from
            // the bucket key produced by run_turn_digest.
            let parts: Vec<&str> = outcome.key.splitn(3, '|').collect();
            if parts.len() != 3 {
                buckets_purge_skipped.push((outcome.key.clone(), "key_split_failed"));
                continue;
            }
            let (repo, year_week, top_topic) = (parts[0], parts[1], parts[2]);
            match runtime.block_on(live.verify_digest_indexed(
                repo,
                year_week,
                top_topic,
                VERIFY_TIMEOUT,
            )) {
                Ok(true) => {
                    if let Some(ids) = bucket_event_ids.get(&outcome.key) {
                        match runtime.block_on(live.delete_source_turns(ids)) {
                            Ok(n) => records_purged += n,
                            Err(e) => {
                                eprintln!("turn-digest: purge {key}: {e}", key = outcome.key);
                            }
                        }
                    }
                }
                Ok(false) => {
                    eprintln!(
                        "turn-digest: purge skipped for {key} — digest not indexed within {VERIFY_TIMEOUT:?}",
                        key = outcome.key
                    );
                    buckets_purge_skipped.push((outcome.key.clone(), "verify_timeout"));
                }
                Err(e) => {
                    eprintln!(
                        "turn-digest: purge skipped for {key} — verify error: {e}",
                        key = outcome.key
                    );
                    buckets_purge_skipped.push((outcome.key.clone(), "verify_error"));
                }
            }
        }
    }

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops turn-digest ({mode_label})");
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
        println!("records_purged:       {records_purged}");
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
            println!("  {label}  {}", o.key);
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
    extras.insert("records_purged".into(), records_purged.into());
    extras.insert("usd_cents".into(), report.usd_cents.into());
    extras.insert("purge_originals".into(), purge_originals.into());
    if !buckets_purge_skipped.is_empty() {
        extras.insert(
            "buckets_purge_skipped".into(),
            serde_json::Value::Array(
                buckets_purge_skipped
                    .iter()
                    .map(|(k, reason)| serde_json::json!({ "key": k, "reason": reason }))
                    .collect(),
            ),
        );
    }
    super::record_sweep_run(
        "turn_digest",
        started_at,
        "success",
        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
            records_dropped: records_purged,
            extras,
            ..Default::default()
        },
    );
    ExitCode::SUCCESS
}

/// phase11w — `cortex-ops tool-call-digest`. Two execution modes:
///
/// 1. **Synthetic preview** (default, `--apply` off) — drives an
///    in-process backend with a deterministic suite (6 Bash, 5 Read,
///    and 4 Edit, all 60 d old). Lets operators verify the
///    `(repo, year_week, tool)` bucketisation contract without
///    touching live state.
/// 2. **Live** (`--apply` on) — paginates Meili's `cortex_tool_calls`
///    index for everything older than the cutoff, runs each bucket
///    through the live `cortex-ingestion` (paired with
///    `--purge-originals` for the `cortex-api /v1/admin/forget`
///    cascade). The cron schedule (`retention.tool_call_digest`)
///    ships this flag set.
#[allow(clippy::too_many_arguments)]
pub(super) fn tool_call_digest(
    time_travel: Option<String>,
    dry_run: bool,
    rebuild: bool,
    budget_cents: u64,
    apply: bool,
    purge_originals: bool,
    max_records: usize,
    _page_size: u32,
    age_days: Option<i64>,
    min_bucket_size: Option<usize>,
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
    if let Some(d) = age_days {
        plan.digest_after_days = d;
    }
    if let Some(m) = min_bucket_size {
        plan.min_bucket_size = m;
    }

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
            let cfg = cortex_config::Config::load().unwrap_or_default();
            let api_base = cfg
                .dashboard
                .api_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:17000".to_string());
            let api_token = cfg.dashboard.api_token.clone();
            let ingestion_base = cfg
                .ingestion
                .ingestion_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:17010".to_string());
            let meili_base = cfg
                .meili
                .meili_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:7700".to_string());
            let meili_key = cfg
                .meili
                .meili_api_key
                .clone()
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
            let fetched = match runtime.block_on(
                super::tool_call_digest_live::fetch_old_tool_calls_via_admin(
                    &live.api_base,
                    live.api_token.as_deref(),
                    cutoff,
                    max_records,
                ),
            ) {
                Ok(rows) => rows,
                Err(e) => {
                    eprintln!("tool-call-digest: admin enumerator: {e:#}");
                    super::record_sweep_run(
                        "tool_call_digest",
                        started_at,
                        "failed",
                        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
                            last_error: Some(
                                format!("admin enumerator: {e}").chars().take(256).collect(),
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
            (
                synthetic,
                Box::new(MemoryToolCallDigestBackend::new()),
                "preview",
            )
        };

    let fetched_count = tool_calls.len();

    // Capture per-bucket event ids before run_tool_call_digest
    // consumes the source vec — the safety-gated purge needs them.
    let mut bucket_event_ids: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    if apply && purge_originals && !dry_run {
        for t in &tool_calls {
            let key = format!(
                "{}|{}|{}",
                t.repo,
                cortex_workers::retention::tool_call_digest::iso_year_week(t.occurred_at),
                t.tool,
            );
            bucket_event_ids
                .entry(key)
                .or_default()
                .push(t.event_id.clone());
        }
    }

    // Build a separate live handle for the post-run verify+purge
    // cascade — the orchestrator's `delete_source_tool_calls` impl
    // is now a no-op, so the actual purge runs externally with a
    // safety gate.
    let live_for_purge: Option<super::tool_call_digest_live::LiveToolCallDigestBackend> =
        if apply && purge_originals && !dry_run {
            let cfg = cortex_config::Config::load().unwrap_or_default();
            let api_base = cfg
                .dashboard
                .api_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:17000".to_string());
            let api_token = cfg.dashboard.api_token.clone();
            let ingestion_base = cfg
                .ingestion
                .ingestion_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:17010".to_string());
            let meili_base = cfg
                .meili
                .meili_url
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:7700".to_string());
            let meili_key = cfg
                .meili
                .meili_api_key
                .clone()
                .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());
            match super::tool_call_digest_live::LiveToolCallDigestBackend::new(
                api_base,
                api_token,
                ingestion_base,
                meili_base,
                meili_key,
            ) {
                Ok(b) => Some(b),
                Err(e) => {
                    eprintln!("tool-call-digest: purge backend init: {e:#}");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            None
        };

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

    // SAFETY GATE — verify+purge cascade outside the orchestrator.
    // The trait impl `delete_source_tool_calls` is a no-op so the
    // orchestrator records every bucket as "digested" without
    // actually deleting; the real purge runs here, gated on each
    // digest envelope being queryable in Meili. Per user constraint
    // 2026-05-05: data may only be removed after summarisation lands.
    let mut records_purged: u64 = 0;
    let mut buckets_purge_skipped: Vec<(String, &'static str)> = Vec::new();
    if let Some(live) = live_for_purge.as_ref() {
        const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
        for outcome in &report.outcomes {
            // A bucket is purge-eligible if its digest exists — either
            // produced in this run (`digested`) or already present from
            // a prior run (`already_digested`). The latter covers the
            // case where an earlier digest landed but the purge cascade
            // never completed (e.g. `--purge-originals` was off, or
            // verify timed out). The verify gate below still requires
            // the digest to be queryable in Meili before forget fires.
            if !outcome.digested && !outcome.already_digested {
                continue;
            }
            let parts: Vec<&str> = outcome.key.splitn(3, '|').collect();
            if parts.len() != 3 {
                buckets_purge_skipped.push((outcome.key.clone(), "key_split_failed"));
                continue;
            }
            let (repo, year_week, tool) = (parts[0], parts[1], parts[2]);
            match runtime.block_on(live.verify_digest_indexed(
                repo,
                year_week,
                tool,
                VERIFY_TIMEOUT,
            )) {
                Ok(true) => {
                    if let Some(ids) = bucket_event_ids.get(&outcome.key) {
                        match runtime.block_on(live.delete_source_tool_calls_external(ids)) {
                            Ok(n) => records_purged += n,
                            Err(e) => {
                                eprintln!("tool-call-digest: purge {key}: {e}", key = outcome.key);
                            }
                        }
                    }
                }
                Ok(false) => {
                    eprintln!(
                        "tool-call-digest: purge skipped for {key} — digest not indexed within {VERIFY_TIMEOUT:?}",
                        key = outcome.key
                    );
                    buckets_purge_skipped.push((outcome.key.clone(), "verify_timeout"));
                }
                Err(e) => {
                    eprintln!(
                        "tool-call-digest: purge skipped for {key} — verify error: {e}",
                        key = outcome.key
                    );
                    buckets_purge_skipped.push((outcome.key.clone(), "verify_error"));
                }
            }
        }
    }

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
        println!("records_purged:       {records_purged}");
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
            println!("  {label}  {}", o.key);
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
    extras.insert("records_purged".into(), records_purged.into());
    extras.insert("usd_cents".into(), report.usd_cents.into());
    extras.insert("purge_originals".into(), purge_originals.into());
    if !buckets_purge_skipped.is_empty() {
        extras.insert(
            "buckets_purge_skipped".into(),
            serde_json::Value::Array(
                buckets_purge_skipped
                    .iter()
                    .map(|(k, reason)| serde_json::json!({ "key": k, "reason": reason }))
                    .collect(),
            ),
        );
    }
    super::record_sweep_run(
        "tool_call_digest",
        started_at,
        "success",
        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
            records_dropped: records_purged,
            extras,
            ..Default::default()
        },
    );
    ExitCode::SUCCESS
}
