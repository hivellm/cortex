//! Phase12g §1 + §2 — `cortex-ops meili-audit` and `cortex-ops meili-reindex`.
//!
//! Both subcommands wrap the `cortex-workers::fulltext` infrastructure
//! the indexer worker already uses. The audit calls
//! [`audit_indexes`] against a `LiveMeiliClient` and prints the
//! per-index doc-count report. The reindex calls
//! [`replay_missing_partitions`] against the same client + a
//! `MeiliFulltextIndexer` so the operator surface is identical to
//! the production worker's boot-time replay path.
//!
//! Exit codes:
//! - `0` — audit reports no drift / reindex completed cleanly.
//! - `2` — audit reports any `empty` / `missing` / `orphan` row,
//!   any backend transport failure, or any reindex error.

use std::process::ExitCode;
use std::sync::Arc;

use cortex_workers::fulltext::audit::{audit_indexes, AuditReport};
use cortex_workers::fulltext::boot_replay::replay_missing_partitions;
use cortex_workers::fulltext::config::FulltextConfig;
use cortex_workers::fulltext::indexer::MeiliFulltextIndexer;
use cortex_workers::fulltext::meili_client::LiveMeiliClient;
use cortex_workers::fulltext::Metrics;

/// `cortex-ops meili-audit` handler.
pub(super) fn meili_audit(json: bool) -> ExitCode {
    let cfg = FulltextConfig::from_env();
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("meili-audit: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let report = match rt.block_on(async {
        let client = LiveMeiliClient::new(&cfg).map_err(|e| format!("meili client: {e}"))?;
        audit_indexes(&client)
            .await
            .map_err(|e| format!("audit: {e}"))
    }) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("meili-audit: {e}");
            return ExitCode::from(2);
        }
    };

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("meili-audit: serialize: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        render_text(&report, &cfg.meili_url);
    }
    if report.has_drift() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

/// `cortex-ops meili-reindex` handler.
///
/// Wraps `cortex_workers::fulltext::boot_replay::replay_missing_partitions`,
/// which is the production worker's boot-time replay path. The CLI
/// surface matches what an operator would expect: pick the archive
/// root (CLI flag → `$CORTEX_ARCHIVE_ROOT` → `<HOME|USERPROFILE>/.cortex/archive`),
/// derive the index prefix from the live `FulltextConfig`, and walk
/// every archive partition Meili does not already cover.
pub(super) fn meili_reindex(archive_root: Option<String>, json: bool) -> ExitCode {
    let cfg = FulltextConfig::from_env();
    let archive_path = match resolve_archive_root(archive_root) {
        Some(p) => p,
        None => {
            eprintln!(
                "meili-reindex: cannot resolve archive root — pass --archive-root \
                 or set CORTEX_ARCHIVE_ROOT / CORTEX_HOME / HOME / USERPROFILE"
            );
            return ExitCode::from(2);
        }
    };
    if !archive_path.exists() {
        eprintln!(
            "meili-reindex: archive root does not exist: {}",
            archive_path.display()
        );
        return ExitCode::from(2);
    }

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("meili-reindex: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let prefix = cfg.index_prefix.clone();
    let result = rt.block_on(async {
        let client = LiveMeiliClient::new(&cfg).map_err(|e| format!("meili client: {e}"))?;
        let client_arc: Arc<dyn cortex_workers::fulltext::meili_client::MeiliClient> =
            Arc::new(client);
        let metrics = Arc::new(Metrics::default());
        let indexer = Arc::new(MeiliFulltextIndexer::new(
            cfg.clone(),
            client_arc.clone(),
            metrics.clone(),
        ));
        replay_missing_partitions(&*client_arc, indexer, &metrics, &archive_path, &prefix)
            .await
            .map_err(|e| format!("replay: {e}"))
    });

    let report = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("meili-reindex: {e}");
            return ExitCode::from(2);
        }
    };

    if json {
        let payload = serde_json::json!({
            "archive_root": archive_path.display().to_string(),
            "prefix": prefix,
            "examined_archives": report.examined_archives,
            "missing_partitions": report.missing_partitions,
            "replayed_events": report.replayed_events,
            "latency_ms": report.latency_ms,
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("meili-reindex: serialize: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        println!("cortex-ops meili-reindex");
        println!("archive_root:        {}", archive_path.display());
        println!("prefix:              {prefix}");
        println!("examined_archives:   {}", report.examined_archives);
        println!("missing_partitions:  {}", report.missing_partitions);
        println!("replayed_events:     {}", report.replayed_events);
        println!("latency_ms:          {}", report.latency_ms);
    }
    ExitCode::SUCCESS
}

fn render_text(report: &AuditReport, meili_url: &str) {
    println!("cortex-ops meili-audit");
    println!("meili_url: {meili_url}");
    println!(
        "summary:   populated={} empty={} missing={} orphan={}",
        report.populated_count, report.empty_count, report.missing_count, report.orphan_count
    );
    println!();
    for row in &report.rows {
        println!(
            "{:<10} {:<32} docs={}",
            row.status, row.index, row.number_of_documents
        );
    }
}

fn resolve_archive_root(cli: Option<String>) -> Option<std::path::PathBuf> {
    if let Some(s) = cli.filter(|s| !s.is_empty()) {
        return Some(std::path::PathBuf::from(s));
    }
    let cfg = cortex_config::Config::load().unwrap_or_default();
    if let Some(s) = cfg.ingestion.archive_root.as_deref() {
        if !s.is_empty() {
            return Some(std::path::PathBuf::from(s));
        }
    }
    if let Some(s) = cfg.ingestion.home.as_deref() {
        if !s.is_empty() {
            return Some(std::path::PathBuf::from(s).join("archive"));
        }
    }
    super::helpers::home_dir().map(|h| h.join(".cortex").join("archive"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_archive_root_prefers_cli_flag() {
        let resolved = resolve_archive_root(Some("E:/explicit".to_string())).unwrap();
        assert_eq!(resolved, std::path::PathBuf::from("E:/explicit"));
    }

    // ADR-016 §3.5 — the env-precedence tests for resolve_archive_root
    // moved to crates/cortex-config/src/load.rs. Per-helper env-
    // mutation tests would duplicate centralised coverage and race
    // each other on CORTEX_ARCHIVE_ROOT / CORTEX_HOME (shared process
    // state). The CLI-flag path stays because it doesn't touch env.
}
