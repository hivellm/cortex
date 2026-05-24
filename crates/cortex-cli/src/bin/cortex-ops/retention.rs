use std::process::ExitCode;

/// Phase9a — `cortex-ops retention-sweep`. Runs one tier-transition
/// pass and exits. Idempotent + concurrency-safe via the
/// `retention_sweeps` table.
///
/// Production Vectorizer integration is wired through the
/// `VectorizerOps` trait; this CLI surface uses the in-memory ops
/// (`MemoryVectorizerOps`) so the dry-run path works without a
/// running Vectorizer server. Live ops integration ships in a
/// follow-up that adds the SDK adapter — keeping the trait surface
/// stable now means that switch is one line.
pub(super) fn retention_sweep(
    time_travel: Option<String>,
    dry_run: bool,
    batch_size: u32,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_storage::MetadataStore;
    use cortex_workers::retention::{run_sweep, MemoryVectorizerOps, SweepError, SweepPlan};

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

    let mut plan = SweepPlan::default_for(now);
    plan.batch_size = batch_size;
    plan.dry_run = dry_run;

    let metadata_path = metadata_db
        .map(std::path::PathBuf::from)
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.ingestion.metadata_db)
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home)
                .join(".cortex")
                .join("metadata.sqlite")
        });

    let store = match MetadataStore::open(&metadata_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("metadata open ({}): {e}", metadata_path.display());
            return ExitCode::FAILURE;
        }
    };

    let sweep_id = cortex_workers::retention::new_sweep_id();
    if let Err(e) = store.start_retention_sweep(&sweep_id, now, 3600) {
        eprintln!("retention-sweep: {e}");
        // Code 2 — another sweep in flight (per spec).
        return ExitCode::from(2);
    }

    // The MemoryVectorizerOps holds an empty store on a fresh CI
    // run, so the dry-run path emits a `0 demoted / 0 dropped` row
    // plus the canonical plan summary. Live Vectorizer integration
    // swaps `ops` for the SDK adapter.
    let ops = MemoryVectorizerOps::new();

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

    let outcome = runtime.block_on(run_sweep(&plan, &ops));
    let finished_at = chrono::Utc::now();
    let (status, exit) = match &outcome {
        Ok(_) => ("success", ExitCode::SUCCESS),
        Err(SweepError::ErrorRateExceeded { .. }) => ("failed", ExitCode::FAILURE),
        Err(SweepError::Vectorizer(_)) => ("failed", ExitCode::FAILURE),
    };

    let report = outcome.unwrap_or_default();
    if let Err(e) = store.finish_retention_sweep(
        &sweep_id,
        finished_at,
        report.records_demoted,
        report.records_dropped,
        &report.tier_transitions_json(),
        status,
    ) {
        eprintln!("retention-sweep: bookkeeping write failed: {e}");
        return ExitCode::FAILURE;
    }

    if json {
        let payload = serde_json::json!({
            "sweep_id": sweep_id,
            "started_at": now.to_rfc3339(),
            "finished_at": finished_at.to_rfc3339(),
            "status": status,
            "dry_run": dry_run,
            "records_demoted": report.records_demoted,
            "records_dropped": report.records_dropped,
            "tier_transitions": report.tier_transitions,
            "transitions": report.transitions,
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops retention-sweep");
        println!("sweep_id:   {sweep_id}");
        println!("now:        {}", now.to_rfc3339());
        println!("dry_run:    {dry_run}");
        println!("status:     {status}");
        println!(
            "demoted:    {}    dropped: {}",
            report.records_demoted, report.records_dropped
        );
        if report.tier_transitions.is_empty() {
            println!("transitions: (none — every collection within thresholds)");
        } else {
            println!("transitions:");
            for (key, count) in &report.tier_transitions {
                println!("  {key}: {count}");
            }
        }
    }
    exit
}

/// Phase11p §1.2 — `cortex-ops sweep-empty` dispatcher. Lists empty
/// Meili indexes (non-canonical legacy + canonical-but-zero) and
/// optionally `DELETE`s them when `--apply` is set. Non-canonical
/// names are dropped immediately; canonical names need explicit
/// approval because empty-canonical can be a transient state right
/// after a settings PATCH but before the first upsert.
pub(super) fn sweep_empty(
    meili: Option<String>,
    meili_key: Option<String>,
    apply: bool,
    json: bool,
) -> ExitCode {
    use cortex_workers::fulltext::meili_client::MeiliClient;
    use cortex_workers::fulltext::sweep::{sweep_empty_canonical, sweep_stale_indexes};
    use cortex_workers::fulltext::{FulltextConfig, LiveMeiliClient};

    let cfg_typed = cortex_config::Config::load().unwrap_or_default();
    let meili_url = meili
        .or_else(|| cfg_typed.meili.meili_url.clone())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let api_key = meili_key
        .or_else(|| cfg_typed.meili.meili_api_key.clone())
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let cfg = FulltextConfig {
        meili_url: meili_url.clone(),
        meili_api_key: api_key,
        ..FulltextConfig::default()
    };
    let client = match LiveMeiliClient::new(&cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sweep-empty: meili client: {e}");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("sweep-empty: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let canonical_empty = match runtime.block_on(sweep_empty_canonical(&client)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("sweep-empty: list canonical empties: {e}");
            return ExitCode::from(2);
        }
    };

    // Plan-only: list non-canonical names alongside canonical empties.
    // sweep_stale_indexes mutates by default (it's the boot-time
    // reaper), so the dry-run path enumerates manually here. Reusing
    // the lib's classifier keeps the predicates honest.
    let indexes = match runtime.block_on(client.list_indexes()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("sweep-empty: list indexes: {e}");
            return ExitCode::from(2);
        }
    };
    use cortex_workers::fulltext::routing::is_canonical_index_name;
    let mut non_canonical_empty: Vec<String> = indexes
        .iter()
        .filter(|i| !is_canonical_index_name(&i.uid) && i.number_of_documents == 0)
        .map(|i| i.uid.clone())
        .collect();
    non_canonical_empty.sort();

    let canonical_empty_uids: Vec<String> = canonical_empty.iter().map(|c| c.uid.clone()).collect();

    if !apply {
        if json {
            let body = serde_json::json!({
                "mode": "dry_run",
                "meili_url": meili_url,
                "non_canonical_empty": non_canonical_empty,
                "canonical_empty": canonical_empty_uids,
                "total_candidates": non_canonical_empty.len() + canonical_empty_uids.len(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string())
            );
        } else {
            println!("cortex-ops sweep-empty (dry-run)");
            println!("meili: {meili_url}");
            println!("non-canonical empty ({}):", non_canonical_empty.len());
            for n in &non_canonical_empty {
                println!("  would drop: {n}");
            }
            println!("canonical empty ({}):", canonical_empty_uids.len());
            for n in &canonical_empty_uids {
                println!("  would drop: {n}");
            }
            println!(
                "total candidates: {}. Pass --apply to delete.",
                non_canonical_empty.len() + canonical_empty_uids.len()
            );
        }
        return ExitCode::SUCCESS;
    }

    // --apply: run the canonical sweep first (it auto-drops non-canonical
    // empties as a side effect of the existing boot-time reaper) and
    // then delete each canonical-empty candidate one by one.
    let stale_report = match runtime.block_on(sweep_stale_indexes(&client)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("sweep-empty: sweep_stale_indexes: {e}");
            return ExitCode::from(2);
        }
    };
    let mut canonical_dropped: Vec<String> = Vec::new();
    let mut canonical_failed: Vec<(String, String)> = Vec::new();
    for c in &canonical_empty {
        match runtime.block_on(client.delete_index(&c.uid)) {
            Ok(()) => canonical_dropped.push(c.uid.clone()),
            Err(e) => canonical_failed.push((c.uid.clone(), e.to_string())),
        }
    }

    if json {
        let body = serde_json::json!({
            "mode": "apply",
            "meili_url": meili_url,
            "non_canonical_dropped": stale_report.deleted_stale_empty,
            "non_canonical_warned_names": stale_report.warned_names,
            "canonical_dropped": canonical_dropped,
            "canonical_failed": canonical_failed
                .iter()
                .map(|(n, e)| serde_json::json!({"uid": n, "error": e}))
                .collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string())
        );
    } else {
        println!("cortex-ops sweep-empty (apply)");
        println!("meili: {meili_url}");
        println!(
            "non-canonical dropped: {} (warned-preserved: {})",
            stale_report.deleted_stale_empty, stale_report.kept_warning,
        );
        println!("canonical dropped: {}", canonical_dropped.len());
        for n in &canonical_dropped {
            println!("  dropped: {n}");
        }
        if !canonical_failed.is_empty() {
            println!("canonical failed: {}", canonical_failed.len());
            for (n, e) in &canonical_failed {
                println!("  failed: {n}: {e}");
            }
        }
    }

    if canonical_failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}
