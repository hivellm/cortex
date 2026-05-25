use std::process::ExitCode;

/// Phase9f — `cortex-ops meili-prune`. Today's surface is a
/// synthetic preview against the in-memory backend; the production
/// pipeline (Meili `update_documents` task await) lands with
/// phase9k's cron scheduler.
pub(super) fn meili_prune(
    time_travel: Option<String>,
    dry_run: bool,
    rebuild: bool,
    batch_size: u32,
    json: bool,
) -> ExitCode {
    use cortex_workers::retention::meili_prune::{
        run_meili_prune, MeiliDoc, MemoryMeiliBackend, PrunePlan,
    };

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

    let mut plan = PrunePlan::default_for(now);
    plan.dry_run = dry_run;
    plan.rebuild = rebuild;
    plan.batch_size = batch_size;

    let backend = MemoryMeiliBackend::new();
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

    // Synthetic preview: 3 turns over 90 d + 1 fresh + 1 oversize.
    let preview_seed = [
        MeiliDoc {
            event_id: "01PREVIEW-T1".to_string(),
            index: "cortex_turns".to_string(),
            occurred_at: now - chrono::Duration::days(91),
            summary: "preview short summary 1".to_string(),
            already_pruned: false,
        },
        MeiliDoc {
            event_id: "01PREVIEW-T2".to_string(),
            index: "cortex_turns".to_string(),
            occurred_at: now - chrono::Duration::days(120),
            summary: "preview short summary 2".to_string(),
            already_pruned: false,
        },
        MeiliDoc {
            event_id: "01PREVIEW-FRESH".to_string(),
            index: "cortex_turns".to_string(),
            occurred_at: now - chrono::Duration::days(5),
            summary: "fresh — should be left alone".to_string(),
            already_pruned: false,
        },
        MeiliDoc {
            event_id: "01PREVIEW-BIG".to_string(),
            index: "cortex_tool_calls".to_string(),
            occurred_at: now - chrono::Duration::days(100),
            summary: "x".repeat(8_000),
            already_pruned: false,
        },
    ];
    runtime.block_on(async {
        backend
            .seed("cortex_turns", preview_seed[..3].to_vec())
            .await;
        backend
            .seed("cortex_tool_calls", Vec::from([preview_seed[3].clone()]))
            .await;
    });

    let report = match runtime.block_on(run_meili_prune(&plan, &backend)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("meili-prune: {e}");
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
        println!("cortex-ops meili-prune (preview)");
        println!("now:               {}", now.to_rfc3339());
        println!("dry_run:           {dry_run}");
        println!("rebuild:           {rebuild}");
        println!("batch_size:        {batch_size}");
        println!("examined:          {}", report.examined);
        println!("pruned:            {}", report.pruned);
        println!("summaries_capped:  {}", report.summaries_capped);
        println!("skipped:           {}", report.skipped);
        for (idx, n) in &report.per_index {
            println!("  {idx}: {n}");
        }
    }
    // phase11v §6 — record one row in `retention_sweeps` so the
    // dashboard's per-card history sees this run.
    let mut extras = serde_json::Map::new();
    extras.insert("examined".into(), report.examined.into());
    extras.insert("pruned".into(), report.pruned.into());
    extras.insert("summaries_capped".into(), report.summaries_capped.into());
    extras.insert("skipped".into(), report.skipped.into());
    super::record_sweep_run(
        "meili_prune",
        started_at,
        "success",
        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
            bytes_reclaimed: 0,
            records_demoted: 0,
            records_dropped: report.pruned as u64,
            last_error: None,
            extras,
        },
    );
    ExitCode::SUCCESS
}
