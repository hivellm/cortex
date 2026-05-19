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
    let cutoff = match DateTime::parse_from_rfc3339(&before) {
        Ok(dt) => dt.with_timezone(&Utc),
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
    extras.insert(
        "files_unreadable".into(),
        report.files_unreadable.into(),
    );
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
    if let Ok(s) = std::env::var("CORTEX_HOME") {
        if !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    home_dir().map(|h| h.join(".cortex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_home_prefers_cli_flag() {
        std::env::remove_var("CORTEX_HOME");
        let resolved = resolve_home(Some("E:/explicit".to_string())).unwrap();
        assert_eq!(resolved, PathBuf::from("E:/explicit"));
    }

    #[test]
    fn resolve_home_falls_back_to_env() {
        std::env::set_var("CORTEX_HOME", "E:/env");
        let resolved = resolve_home(None).unwrap();
        assert_eq!(resolved, PathBuf::from("E:/env"));
        std::env::remove_var("CORTEX_HOME");
    }

    #[test]
    fn resolve_home_drops_empty_strings_through_to_default() {
        // Empty CLI flag value shouldn't pin the resolver to "" —
        // the filter step inside `resolve_home` strips it before the
        // env lookup runs.
        std::env::set_var("CORTEX_HOME", "E:/from-env");
        let resolved = resolve_home(Some("".to_string())).unwrap();
        assert_eq!(resolved, PathBuf::from("E:/from-env"));
        std::env::remove_var("CORTEX_HOME");
    }
}
