//! `cortex-ops sessions-backfill` — populate the `sessions` table
//! from the parquet archive.
//!
//! ## Why
//!
//! The ingestion router writes every envelope to the archive but
//! never inserts a corresponding row in `sessions`. The consolidator
//! nightly enumerates the 24h window from `sessions` and finds zero
//! rows, so the daily cron produces zero summaries. The only way
//! consolidations were landing in production was via the manual
//! `cortex-consolidator nightly --all` back-fill which walks the
//! archive directly.
//!
//! This subcommand closes the gap: walk every envelope in the
//! archive, group by `session_id`, and `upsert_session` for each
//! distinct session with its earliest `occurred_at` as
//! `started_at`. The upsert is idempotent — running hourly via cron
//! keeps the table in sync without rewriting rows whose `tool` /
//! `model` / `repo` already match.
//!
//! Once the table is populated the nightly cron's
//! `enumerate_recent_sessions` query returns the last-24h slice and
//! the consolidator processes each session into one
//! `Kind::Consolidation` summary per night.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{DateTime, Utc};
use cortex_storage::{archive::walk_envelopes, MetadataStore};
use serde_json::json;

use super::helpers::resolve_metadata_db_path;

/// One aggregated session pre-upsert.
#[derive(Debug, Clone)]
struct SessionRecord {
    tool: String,
    model: Option<String>,
    repo: Option<String>,
    user: Option<String>,
    started_at: DateTime<Utc>,
    envelope_count: u32,
}

/// `cortex-ops sessions-backfill` entry point.
pub(super) fn sessions_backfill(
    metadata_db: Option<String>,
    archive_root: Option<String>,
    dry_run: bool,
    json_output: bool,
) -> ExitCode {
    let archive = resolve_archive_root(archive_root);
    let db_path = metadata_db
        .map(PathBuf::from)
        .unwrap_or_else(resolve_metadata_db_path);

    if !archive.exists() {
        eprintln!(
            "sessions-backfill: archive root {} not found",
            archive.display()
        );
        return ExitCode::FAILURE;
    }

    // 1. Walk the archive and aggregate by session_id. The predicate
    //    returns false so `walk_envelopes` does not retain envelopes
    //    in its return Vec (memory-safe for any archive size).
    let mut sessions: BTreeMap<String, SessionRecord> = BTreeMap::new();
    let mut total_envelopes: u64 = 0;
    let walk_result = walk_envelopes(&archive, |env| {
        total_envelopes = total_envelopes.saturating_add(1);
        let sid = env.session_id.trim();
        if sid.is_empty() {
            return false;
        }
        let occurred = match DateTime::parse_from_rfc3339(&env.occurred_at) {
            Ok(t) => t.with_timezone(&Utc),
            // Skip envelopes with malformed timestamps rather than
            // synthesising a `now` value — every other path in
            // Cortex insists on RFC-3339 and silently substituting
            // one would lose the bug signal.
            Err(_) => return false,
        };
        let tool = env.tool.trim();
        let repo = env.context.repo.clone().filter(|s| !s.is_empty());
        let user = env.context.user.clone().filter(|s| !s.is_empty());
        let model = env.model.clone().filter(|s| !s.is_empty());

        sessions
            .entry(sid.to_string())
            .and_modify(|rec| {
                rec.envelope_count = rec.envelope_count.saturating_add(1);
                if occurred < rec.started_at {
                    rec.started_at = occurred;
                }
                // First non-empty wins for the small set of
                // identity fields. A later envelope without `model`
                // never clobbers a value the session already has.
                if rec.tool.is_empty() && !tool.is_empty() {
                    rec.tool = tool.to_string();
                }
                if rec.model.is_none() && model.is_some() {
                    rec.model = model.clone();
                }
                if rec.repo.is_none() && repo.is_some() {
                    rec.repo = repo.clone();
                }
                if rec.user.is_none() && user.is_some() {
                    rec.user = user.clone();
                }
            })
            .or_insert_with(|| SessionRecord {
                tool: tool.to_string(),
                model,
                repo,
                user,
                started_at: occurred,
                envelope_count: 1,
            });
        false
    });
    if let Err(e) = walk_result {
        eprintln!("sessions-backfill: archive walk failed: {e}");
        return ExitCode::FAILURE;
    }

    let sessions_seen: u32 = u32::try_from(sessions.len()).unwrap_or(u32::MAX);

    // 2. Upsert.
    let mut upserted: u32 = 0;
    let mut failed: u32 = 0;
    if !dry_run {
        let store = match MetadataStore::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "sessions-backfill: open metadata DB {}: {e}",
                    db_path.display()
                );
                return ExitCode::FAILURE;
            }
        };
        for (sid, rec) in &sessions {
            match store.upsert_session(
                sid,
                &rec.tool,
                rec.model.as_deref(),
                rec.repo.as_deref(),
                rec.user.as_deref(),
                rec.started_at,
            ) {
                Ok(()) => upserted = upserted.saturating_add(1),
                Err(e) => {
                    failed = failed.saturating_add(1);
                    eprintln!("sessions-backfill: upsert {sid} failed: {e}");
                }
            }
        }
    }

    if json_output {
        let payload = json!({
            "archive_root": archive.display().to_string(),
            "metadata_db": db_path.display().to_string(),
            "dry_run": dry_run,
            "envelopes_walked": total_envelopes,
            "sessions_seen": sessions_seen,
            "sessions_upserted": upserted,
            "sessions_failed": failed,
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("sessions-backfill: serialise: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops sessions-backfill");
        println!("archive_root:       {}", archive.display());
        println!("metadata_db:        {}", db_path.display());
        println!("dry_run:            {dry_run}");
        println!("envelopes_walked:   {total_envelopes}");
        println!("sessions_seen:      {sessions_seen}");
        println!("sessions_upserted:  {upserted}");
        if failed > 0 {
            println!("sessions_failed:    {failed}");
        }
    }

    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Resolve the archive root: explicit flag → `CORTEX_ARCHIVE_ROOT`
/// env → `<home>/.cortex/archive`. Mirrors the consolidator's
/// resolution so both walk the same parquet tree.
fn resolve_archive_root(arg: Option<String>) -> PathBuf {
    if let Some(p) = arg {
        return PathBuf::from(p);
    }
    let cfg = cortex_config::Config::load().unwrap_or_default();
    if let Some(p) = cfg.ingestion.archive_root.as_deref() {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(home) = cfg.ingestion.home.as_deref() {
        if !home.is_empty() {
            return PathBuf::from(home).join("archive");
        }
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".cortex").join("archive")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_archive_root_prefers_explicit_flag() {
        let p = resolve_archive_root(Some("D:/explicit".into()));
        assert_eq!(p, PathBuf::from("D:/explicit"));
    }

    #[test]
    fn resolve_archive_root_falls_back_to_cortex_home() {
        let saved_arch = std::env::var_os("CORTEX_ARCHIVE_ROOT");
        let saved_home = std::env::var_os("CORTEX_HOME");
        std::env::remove_var("CORTEX_ARCHIVE_ROOT");
        std::env::set_var("CORTEX_HOME", "D:/fakehome");
        let p = resolve_archive_root(None);
        match saved_arch {
            Some(v) => std::env::set_var("CORTEX_ARCHIVE_ROOT", v),
            None => std::env::remove_var("CORTEX_ARCHIVE_ROOT"),
        }
        match saved_home {
            Some(v) => std::env::set_var("CORTEX_HOME", v),
            None => std::env::remove_var("CORTEX_HOME"),
        }
        assert_eq!(p, PathBuf::from("D:/fakehome").join("archive"));
    }
}
