//! Optional bounded archive projection. Explicit daily memories never use it.

use std::path::{Path, PathBuf};
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

use cortex_storage::archive_index::{ArchiveIndex, ArchivePollReport};
use serde::Serialize;

use super::{archive_loader::envelope_to_hit, KeywordRequest, LaneHit, MemoryKeywordLane};
use crate::LoaderMetrics;

const INDEXES: [&str; 3] = ["cortex-code", "cortex-docs", "cortex-decisions"];

/// Observed archive state, separate from daily-memory availability.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveLoaderStatus {
    /// Explicit operator opt-in, including the first bounded historical backfill.
    pub enabled: bool,
    /// Disabled, opening, historical_backfill, tailing, or degraded.
    pub phase: &'static str,
    /// Where this projection is visible. Live Meili search is independent.
    pub visibility: &'static str,
    /// Latest committed and published bounded poll, when available.
    pub last_poll: Option<ArchivePollReport>,
    /// Fixed failure stage; source contents never appear here.
    pub last_error: Option<&'static str>,
    /// Number of retained hits in the bounded in-memory snapshot.
    pub snapshot_hits: usize,
}

impl Default for ArchiveLoaderStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            phase: "disabled",
            visibility: "dashboard_and_memory_keyword_fallback; live_meili_is_independent",
            last_poll: None,
            last_error: None,
            snapshot_hits: 0,
        }
    }
}

/// Configure the loader. A root alone does not authorize historical backfill.
/// Each iteration owns one blocking job; timers cannot overlap decoding work.
pub fn start(
    enabled: bool,
    root: Option<PathBuf>,
    metadata_path: PathBuf,
    lane: Arc<MemoryKeywordLane>,
    metrics: Arc<LoaderMetrics>,
    refresh: Duration,
) {
    if !enabled {
        return;
    }
    if let Ok(mut status) = lane.archive_status.lock() {
        status.enabled = true;
        status.phase = "opening";
    }
    let Some(root) = root else {
        record_error(&lane, "archive_root_missing");
        return;
    };
    tokio::spawn(async move {
        let mut index = None;
        loop {
            let root = root.clone();
            let metadata_path = metadata_path.clone();
            let worker_lane = lane.clone();
            let worker_metrics = metrics.clone();
            let task = tokio::task::spawn_blocking(move || {
                let _publication = match worker_lane.archive_update.lock() {
                    Ok(guard) => guard,
                    Err(_) => {
                        record_error(&worker_lane, "archive_publication_lock_failed");
                        return None;
                    }
                };
                let mut index = match index {
                    Some(index) => index,
                    None => match ArchiveIndex::open(&root, &metadata_path) {
                        Ok(index) => index,
                        Err(error) => {
                            tracing::warn!(%error, "incremental archive open failed");
                            record_error(&worker_lane, "archive_open_failed");
                            return None;
                        }
                    },
                };
                if let Err(error) = poll_and_publish(&mut index, &worker_lane, &worker_metrics) {
                    tracing::warn!(%error, "incremental archive poll failed");
                    record_error(&worker_lane, "archive_poll_failed");
                    // Reopen from durable checkpoints, never reuse a decoder
                    // whose proposed batch may have failed to commit.
                    return None;
                }
                Some(index)
            });
            index = match task.await {
                Ok(index) => index,
                Err(error) => {
                    tracing::warn!(%error, "incremental archive worker failed");
                    record_error(&lane, "archive_worker_failed");
                    None
                }
            };
            tokio::time::sleep(refresh.max(Duration::from_secs(1))).await;
        }
    });
}

fn record_error(lane: &MemoryKeywordLane, stage: &'static str) {
    if let Ok(mut status) = lane.archive_status.lock() {
        status.phase = "degraded";
        status.last_error = Some(stage);
    }
}

/// Caller holds the publication lock shared with explicit forgetting.
fn poll_and_publish(
    index: &mut ArchiveIndex,
    lane: &MemoryKeywordLane,
    metrics: &LoaderMetrics,
) -> anyhow::Result<()> {
    // Older persisted hits omitted access labels. Hide them before publication
    // and rebuild through the same bounded decoder used for ordinary backfill.
    index.require_projection_version("lane-hit-acl-v1")?;
    let report = index.poll(|envelope| {
        envelope_to_hit(envelope).and_then(|hit| serde_json::to_value(hit).ok())
    })?;
    let hits = index
        .snapshot()?
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<LaneHit>, _>>()?;
    let count = hits.len();
    publish_snapshot(lane, hits)?;
    metrics.record_archive_refresh_now();
    metrics.add_archive_envelopes_seeded("incremental", report.projections_written as u64);
    metrics.record_archive_incremental_poll(&report);
    let mut status = lane
        .archive_status
        .lock()
        .map_err(|_| anyhow::anyhow!("archive status lock poisoned"))?;
    status.phase = if !report.errors.is_empty() {
        "degraded"
    } else if report.historical_backfill {
        "historical_backfill"
    } else {
        "tailing"
    };
    status.last_error = (!report.errors.is_empty()).then_some("archive_source_errors");
    status.snapshot_hits = count;
    status.last_poll = Some(report);
    Ok(())
}

fn publish_snapshot(lane: &MemoryKeywordLane, hits: Vec<LaneHit>) -> anyhow::Result<()> {
    let mut snapshots = lane
        .hits
        .lock()
        .map_err(|_| anyhow::anyhow!("archive lane lock poisoned"))?;
    if INDEXES
        .iter()
        .all(|index| snapshots.get(*index) == Some(&hits))
    {
        return Ok(());
    }
    for index in INDEXES {
        snapshots.insert(index.to_string(), hits.clone());
    }
    lane.archive_revision.fetch_add(1, Ordering::AcqRel);
    Ok(())
}

/// The production strategies use scoped aliases; the archive owns three shared
/// bounded snapshots. Resolve them without duplicating each snapshot per repo.
pub(super) fn scoped_archive_hits(
    snapshots: &std::collections::BTreeMap<String, Vec<LaneHit>>,
    request: &KeywordRequest,
) -> Vec<LaneHit> {
    let Some(repo) = request
        .scope
        .repo
        .as_deref()
        .filter(|repo| !repo.is_empty())
    else {
        return Vec::new();
    };
    let slug = cortex_storage::names::slug_for_repo(repo);
    let Some(family) = ["code", "docs", "decisions"]
        .into_iter()
        .find(|family| request.index == format!("cortex-{slug}-{family}"))
    else {
        return Vec::new();
    };
    let query: String = request.query.chars().take(4096).collect();
    let terms: Vec<_> = query
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|term| !term.is_empty())
        .take(32)
        .map(str::to_lowercase)
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<_> = snapshots
        .get(&format!("cortex-{family}"))
        .into_iter()
        .flatten()
        // Slugs address indexes; only exact identities authorize repository data.
        .filter(|hit| hit.repo.as_deref() == Some(repo))
        .filter_map(|hit| {
            let text = hit.text.to_lowercase();
            let matches = terms
                .iter()
                .filter(|term| text.contains(term.as_str()))
                .count();
            (matches > 0).then(|| {
                let mut hit = hit.clone();
                hit.score = matches as f64 / terms.len() as f64;
                hit
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.ts.cmp(&a.ts))
            .then(a.doc_id.cmp(&b.doc_id))
    });
    hits.truncate(request.limit);
    hits
}

/// Persist suppression and clear the local projection after a confirmed forget.
/// Serialization prevents an older in-flight snapshot from resurrecting a hit.
pub fn forget_projection(
    metadata_path: &Path,
    event_id: &str,
    lane: Option<&MemoryKeywordLane>,
) -> anyhow::Result<()> {
    let _publication = lane
        .map(|lane| lane.archive_update.lock())
        .transpose()
        .map_err(|_| anyhow::anyhow!("archive publication lock poisoned"))?;
    let conn = rusqlite::Connection::open(metadata_path)?;
    conn.busy_timeout(Duration::from_millis(100))?;
    cortex_storage::archive_index::suppress_archive_event(&conn, event_id)?;
    if let Some(lane) = lane {
        let mut snapshots = lane
            .hits
            .lock()
            .map_err(|_| anyhow::anyhow!("archive lane lock poisoned"))?;
        let archive_id = format!("archive|{event_id}");
        for hits in snapshots.values_mut() {
            hits.retain(|hit| hit.doc_id != archive_id && hit.doc_id != event_id);
        }
        if let Ok(mut status) = lane.archive_status.lock() {
            status.snapshot_hits = snapshots.get(INDEXES[0]).map_or(0, Vec::len);
        }
        lane.archive_revision.fetch_add(1, Ordering::AcqRel);
    }
    Ok(())
}

#[cfg(test)]
#[path = "archive_incremental_tests.rs"]
mod tests;
