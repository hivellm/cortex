use super::helpers::home_dir;
use std::process::ExitCode;

/// Phase9c — `cortex-ops cas-vacuum`. Drops orphaned CAS blobs and
/// reclaims disk via SQLite `VACUUM` when the freelist warrants it.
pub(super) fn cas_vacuum(
    time_travel: Option<String>,
    dry_run: bool,
    force: bool,
    cas_db: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_workers::retention::cas_vacuum::{open_store, run, VacuumError, VacuumOpts};

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

    let cas_path = cas_db
        .map(std::path::PathBuf::from)
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.ingestion.cas_db)
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex/cas.sqlite"))
                .unwrap_or_else(|| std::path::PathBuf::from(".cortex/cas.sqlite"))
        });

    let mut store = match open_store(&cas_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cas-vacuum: open ({}): {e}", cas_path.display());
            return ExitCode::FAILURE;
        }
    };

    let mut opts = VacuumOpts::default_for(now);
    opts.dry_run = dry_run;
    opts.force = force;

    match run(&mut store, &opts) {
        Ok(report) => {
            if json {
                match serde_json::to_string_pretty(&report) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("serialize: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                println!("cortex-ops cas-vacuum");
                println!("now:           {}", now.to_rfc3339());
                println!("cas_db:        {}", cas_path.display());
                println!("dry_run:       {dry_run}");
                println!("total_blobs:   {}", report.total_blobs);
                println!("dropped:       {}", report.blobs_dropped);
                println!("reclaimed:     {} B", report.bytes_reclaimed);
                println!(
                    "free_ratio:    {:.2} (vacuum={})",
                    report.free_pages_ratio, report.did_vacuum
                );
                if report.did_vacuum {
                    println!("vacuum_ms:     {}", report.vacuum_ms);
                }
                if report.safeguard_tripped {
                    println!("WARN safeguard would trip on a live run (>50 % of total)");
                }
            }
            let mut extras = serde_json::Map::new();
            extras.insert("total_blobs".into(), report.total_blobs.into());
            extras.insert("did_vacuum".into(), report.did_vacuum.into());
            extras.insert("safeguard_tripped".into(), report.safeguard_tripped.into());
            super::record_sweep_run(
                "cas_vacuum",
                started_at,
                "success",
                cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
                    bytes_reclaimed: report.bytes_reclaimed,
                    records_dropped: report.blobs_dropped,
                    extras,
                    ..Default::default()
                },
            );
            ExitCode::SUCCESS
        }
        Err(VacuumError::SafeguardTripped {
            would_drop,
            total_blobs,
        }) => {
            eprintln!(
                "cas-vacuum: safeguard tripped — would_drop={would_drop} > 50 % of total_blobs={total_blobs}; pass --force to override"
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("cas-vacuum: {e}");
            ExitCode::FAILURE
        }
    }
}
