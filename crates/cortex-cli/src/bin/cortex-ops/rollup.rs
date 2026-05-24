use super::{helpers::home_dir, RollupGranularityArg};
use std::process::ExitCode;

/// Phase9b — `cortex-ops rollup`. Compacts the archive's hourly /
/// daily / monthly Parquet partitions per spec 19. Quarantines
/// `*.corrupted*` and orphan `*.tmp` files on entry so the working
/// tree is clean before compaction starts.
pub(super) fn rollup(
    time_travel: Option<String>,
    dry_run: bool,
    granularity: RollupGranularityArg,
    archive_root: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_workers::retention::parquet_rollup::{
        apply_three_year_drop, compact_partition, enumerate_compactable, quarantine_pre_existing,
        Granularity, RollupCounts,
    };

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

    let archive = archive_root
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.ingestion.archive_root)
        })
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex/archive").display().to_string())
                .unwrap_or_else(|| ".cortex/archive".to_string())
        });
    let archive_path = std::path::PathBuf::from(&archive);

    let granularities: Vec<Granularity> = match granularity {
        RollupGranularityArg::All => vec![
            Granularity::HourlyToDaily,
            Granularity::DailyToMonthly,
            Granularity::ThreeYearDrop,
        ],
        RollupGranularityArg::HourlyToDaily => vec![Granularity::HourlyToDaily],
        RollupGranularityArg::DailyToMonthly => vec![Granularity::DailyToMonthly],
        RollupGranularityArg::ThreeYearDrop => vec![Granularity::ThreeYearDrop],
    };

    let mut totals = RollupCounts::default();
    // Pre-flight: quarantine corrupted + orphan tmp files.
    let qcounts = if dry_run {
        RollupCounts::default()
    } else {
        quarantine_pre_existing(&archive_path)
    };
    totals.merge(&qcounts);

    let mut per_granularity: Vec<(Granularity, RollupCounts, usize)> = Vec::new();
    let mut had_error = false;
    for g in granularities {
        let plans = enumerate_compactable(&archive_path, now, g);
        let plan_count = plans.len();
        let mut sub = RollupCounts::default();
        if !dry_run {
            for plan in &plans {
                let result = match g {
                    Granularity::ThreeYearDrop => apply_three_year_drop(&archive_path, plan),
                    _ => compact_partition(&archive_path, plan),
                };
                match result {
                    Ok(c) => sub.merge(&c),
                    Err(e) => {
                        had_error = true;
                        tracing::warn!(error = %e, "rollup: partition compaction failed");
                    }
                }
            }
        }
        per_granularity.push((g, sub.clone(), plan_count));
        totals.merge(&sub);
    }

    if json {
        let payload = serde_json::json!({
            "now": now.to_rfc3339(),
            "archive_root": archive,
            "dry_run": dry_run,
            "totals": totals,
            "per_granularity": per_granularity
                .iter()
                .map(|(g, c, n)| serde_json::json!({
                    "granularity": g.as_str(),
                    "plans": *n,
                    "counts": c,
                }))
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
        println!("cortex-ops rollup");
        println!("now:           {}", now.to_rfc3339());
        println!("archive_root:  {archive}");
        println!("dry_run:       {dry_run}");
        println!(
            "totals:        files_in={} files_out={} reclaimed={}B quarantined={} preserved={} dropped={}",
            totals.files_in,
            totals.files_out,
            totals.bytes_reclaimed,
            totals.quarantined,
            totals.records_preserved,
            totals.records_dropped,
        );
        for (g, c, plans) in &per_granularity {
            println!(
                "  {:<18} plans={plans} files_in={} files_out={} reclaimed={}B preserved={} dropped={}",
                g.as_str(),
                c.files_in,
                c.files_out,
                c.bytes_reclaimed,
                c.records_preserved,
                c.records_dropped,
            );
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
