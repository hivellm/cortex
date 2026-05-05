use std::process::ExitCode;

/// Phase9d — `cortex-ops pii-enforce`. Today's surface is a
/// dry-run probe against the documented cohort matrix; the live
/// backend wiring (Vectorizer / Meili / CAS / classifier) lands
/// with phase9k's cron scheduler. The CLI prints the cohort
/// assignment for a synthetic suite so operators can verify the
/// matcher logic against the spec ladder before the production
/// run executes.
pub(super) fn pii_enforce(
    time_travel: Option<String>,
    dry_run: bool,
    cohort: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_workers::retention::pii_enforce::{
        run_enforcement, EnforcementPlan, MemoryPiiBackend, PiiCohort, PiiRisk, PiiTarget,
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

    let mut plan = EnforcementPlan::default_for(now);
    plan.dry_run = dry_run;
    plan.cohort_filter = match cohort.as_deref() {
        None => None,
        Some("high") => Some(PiiCohort::High30d),
        Some("medium") => Some(PiiCohort::Medium90d),
        Some("null") | Some("null_safety") => Some(PiiCohort::NullSafety90d),
        Some(other) => {
            eprintln!("--cohort: unknown value `{other}` (expected high|medium|null)");
            return ExitCode::FAILURE;
        }
    };

    // Synthetic preview suite: one record per cohort + a fresh
    // record (no-op) + an already-redacted record (idempotence).
    // The synthetic shape lets operators verify the matcher
    // contract without a live archive read; the production walker
    // lands with phase9k.
    let targets = vec![
        PiiTarget {
            event_id: "01PREVIEW-HIGH".to_string(),
            kind: "turn".to_string(),
            pii_risk: Some(PiiRisk::High),
            occurred_at: now - chrono::Duration::days(31),
            body_ref: Some("sha256:preview-high".to_string()),
            redacted: None,
        },
        PiiTarget {
            event_id: "01PREVIEW-MEDIUM".to_string(),
            kind: "turn".to_string(),
            pii_risk: Some(PiiRisk::Medium),
            occurred_at: now - chrono::Duration::days(91),
            body_ref: Some("sha256:preview-medium".to_string()),
            redacted: None,
        },
        PiiTarget {
            event_id: "01PREVIEW-NULL".to_string(),
            kind: "turn".to_string(),
            pii_risk: None,
            occurred_at: now - chrono::Duration::days(95),
            body_ref: Some("sha256:preview-null".to_string()),
            redacted: None,
        },
        PiiTarget {
            event_id: "01PREVIEW-FRESH".to_string(),
            kind: "turn".to_string(),
            pii_risk: Some(PiiRisk::High),
            occurred_at: now - chrono::Duration::days(5),
            body_ref: None,
            redacted: None,
        },
        PiiTarget {
            event_id: "01PREVIEW-DONE".to_string(),
            kind: "turn".to_string(),
            pii_risk: Some(PiiRisk::High),
            occurred_at: now - chrono::Duration::days(200),
            body_ref: None,
            redacted: Some("pii_high_30d".to_string()),
        },
    ];

    let backend = MemoryPiiBackend::new();
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
    let report = match runtime.block_on(run_enforcement(&plan, &backend, targets)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("pii-enforce: {e}");
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
        println!("cortex-ops pii-enforce (preview)");
        println!("now:           {}", now.to_rfc3339());
        println!("dry_run:       {dry_run}");
        println!("examined:      {}", report.examined);
        println!("applied:       {}", report.applied);
        println!("skipped:       {}", report.skipped);
        println!("warnings:      {}", report.null_safety_warnings);
        if !report.cohort_counts.is_empty() {
            println!("cohort counts:");
            for (k, v) in &report.cohort_counts {
                println!("  {k}: {v}");
            }
        }
    }
    let mut extras = serde_json::Map::new();
    extras.insert("examined".into(), report.examined.into());
    extras.insert("applied".into(), report.applied.into());
    extras.insert("skipped".into(), report.skipped.into());
    extras.insert(
        "null_safety_warnings".into(),
        report.null_safety_warnings.into(),
    );
    super::record_sweep_run(
        "pii_enforce",
        started_at,
        "success",
        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
            records_demoted: report.applied as u64,
            extras,
            ..Default::default()
        },
    );
    ExitCode::SUCCESS
}
