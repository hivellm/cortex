//! Phase12b §2 — `cortex-ops retention-archive-purge` handler.
//!
//! Thin CLI shim over [`cortex_storage::archive_purge::purge_before`].
//! Resolves the archive home (CLI flag → `$CORTEX_HOME` →
//! `<HOME|USERPROFILE>/.cortex`), parses the RFC-3339 cutoff,
//! delegates to the storage-layer walker, and prints the JSON
//! report. Exit code semantics match the proposal:
//! - `0` — every classifiable file was either kept or deleted
//!   cleanly (`files_unreadable == 0`).
//! - `2` — at least one file was unreadable or the per-file delete
//!   failed (the partial-failure shape phase12b §2.2 calls out).

use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{DateTime, Utc};

use super::helpers::home_dir;
use super::record_sweep_run;

pub(super) fn run(
    before: String,
    dry_run: bool,
    repo: Option<String>,
    home: Option<String>,
) -> ExitCode {
    let cutoff = match parse_cutoff(&before, Utc::now()) {
        Ok(dt) => dt,
        Err(e) => {
            eprintln!("retention-archive-purge: --before parse: {e}");
            return ExitCode::from(2);
        }
    };
    let home_path: PathBuf = match resolve_home(home) {
        Some(p) => p,
        None => {
            eprintln!(
                "retention-archive-purge: cannot resolve archive home — \
                 pass --home or set CORTEX_HOME / HOME / USERPROFILE"
            );
            return ExitCode::from(2);
        }
    };

    let started_at = Utc::now();
    let report = match cortex_storage::archive_purge::purge_before(
        &home_path,
        cutoff,
        dry_run,
        repo.as_deref(),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("retention-archive-purge: {e}");
            // Phase12b §3.3 — bookkeep the failure so the dashboard's
            // retention card surfaces "last_status: failed" instead of
            // displaying stale "ok" from the previous run.
            record_sweep_run(
                "retention.archive_purge",
                started_at,
                "failed",
                cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
                    bytes_reclaimed: 0,
                    records_demoted: 0,
                    records_dropped: 0,
                    last_error: Some(e.to_string()),
                    extras: serde_json::Map::new(),
                },
            );
            return ExitCode::from(2);
        }
    };

    match serde_json::to_string_pretty(&report) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("retention-archive-purge: serialize: {e}");
            return ExitCode::from(2);
        }
    }

    // Phase12b §3.3 — write one `retention_sweeps` row per invocation
    // so the dashboard sees this sweep alongside every other one
    // (consistent with the bookkeeping shipped in phase11v §6). The
    // `tier_transitions_json` column carries the full PurgeReport so
    // operators can drill into per-run counters via the dashboard.
    let mut extras = serde_json::Map::new();
    extras.insert("files_deleted".into(), report.files_deleted.into());
    extras.insert("files_kept".into(), report.files_kept.into());
    extras.insert("files_partial".into(), report.files_partial.into());
    extras.insert("files_unreadable".into(), report.files_unreadable.into());
    extras.insert(
        "partitions_visited".into(),
        report.partitions_visited.into(),
    );
    extras.insert("dry_run".into(), report.dry_run.into());
    extras.insert("cutoff".into(), report.cutoff.clone().into());
    if let Some(ref repo_filter) = report.repo_filter {
        extras.insert("repo_filter".into(), repo_filter.clone().into());
    }
    let status = if report.files_unreadable > 0 {
        "failed"
    } else {
        "success"
    };
    record_sweep_run(
        "retention.archive_purge",
        started_at,
        status,
        cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
            bytes_reclaimed: report.bytes_reclaimed,
            records_demoted: 0,
            records_dropped: report.files_deleted,
            last_error: None,
            extras,
        },
    );

    if report.files_unreadable > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn resolve_home(cli_home: Option<String>) -> Option<PathBuf> {
    if let Some(s) = cli_home.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(s));
    }
    if let Some(s) = cortex_config::Config::load()
        .ok()
        .and_then(|c| c.ingestion.home)
    {
        if !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    home_dir().map(|h| h.join(".cortex"))
}

/// Resolve the `--before` cutoff. Accepts either an absolute RFC-3339
/// timestamp (`2025-06-21T00:00:00Z`) or a relative duration shorthand
/// (`365d`, `90d`, `4w`, `12h`), in which case the cutoff is `now - dur`.
///
/// The relative form is what the `retention_daemon` cron row passes
/// (`--before 365d`); resolving it here lets the registered command run
/// unchanged instead of failing the RFC-3339 parse (which the scheduler
/// mislabels as `lock_held` via exit code 2).
fn parse_cutoff(before: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(before) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Some(dur) = parse_relative_duration(before) {
        return Ok(now - dur);
    }
    Err(format!(
        "{before:?} is neither RFC-3339 nor a relative duration (Nd/Nw/Nh)"
    ))
}

/// Parse a relative duration shorthand: `<N><unit>` where `unit` is
/// `d` (days), `w` (weeks), or `h` (hours). Returns `None` for any
/// other shape so [`parse_cutoff`] can fall through to an error.
fn parse_relative_duration(s: &str) -> Option<chrono::Duration> {
    let s = s.trim();
    let split = s.len().checked_sub(1)?;
    let (num, unit) = s.split_at(split);
    let n: i64 = num.parse().ok()?;
    if n < 0 {
        return None;
    }
    match unit {
        "d" | "D" => Some(chrono::Duration::days(n)),
        "w" | "W" => Some(chrono::Duration::weeks(n)),
        "h" | "H" => Some(chrono::Duration::hours(n)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_home_prefers_cli_flag() {
        let resolved = resolve_home(Some("E:/explicit".to_string())).unwrap();
        assert_eq!(resolved, PathBuf::from("E:/explicit"));
    }

    // ADR-016 §3.5 — the env-precedence tests for resolve_home moved
    // to crates/cortex-config/src/load.rs (env_overrides_default_*).
    // Cortex-CLI helpers now thread through `Config::load()`, so per-
    // helper env-mutation tests would duplicate centralised coverage
    // and race each other when run in parallel (CORTEX_HOME is shared
    // process state). The CLI-flag path stays here because it doesn't
    // touch env.

    #[test]
    fn parse_cutoff_accepts_rfc3339() {
        let now = DateTime::parse_from_rfc3339("2026-06-21T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let got = parse_cutoff("2025-01-02T03:04:05Z", now).unwrap();
        assert_eq!(got.to_rfc3339(), "2025-01-02T03:04:05+00:00");
    }

    #[test]
    fn parse_cutoff_resolves_relative_days() {
        // The exact cron shape that was failing: `--before 365d`.
        let now = DateTime::parse_from_rfc3339("2026-06-21T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let got = parse_cutoff("365d", now).unwrap();
        assert_eq!(got, now - chrono::Duration::days(365));
    }

    #[test]
    fn parse_relative_duration_units() {
        assert_eq!(
            parse_relative_duration("90d"),
            Some(chrono::Duration::days(90))
        );
        assert_eq!(
            parse_relative_duration("4w"),
            Some(chrono::Duration::weeks(4))
        );
        assert_eq!(
            parse_relative_duration("12h"),
            Some(chrono::Duration::hours(12))
        );
        assert_eq!(
            parse_relative_duration("0d"),
            Some(chrono::Duration::days(0))
        );
    }

    #[test]
    fn parse_relative_duration_rejects_garbage() {
        assert_eq!(parse_relative_duration(""), None);
        assert_eq!(parse_relative_duration("d"), None);
        assert_eq!(parse_relative_duration("365"), None);
        assert_eq!(parse_relative_duration("-5d"), None);
        assert_eq!(parse_relative_duration("30y"), None);
    }

    #[test]
    fn parse_cutoff_rejects_unparseable() {
        let now = Utc::now();
        assert!(parse_cutoff("not-a-date", now).is_err());
    }
}
