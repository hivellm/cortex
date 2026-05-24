//! Phase9c — CAS vacuum.
//!
//! Spec 02 §"CAS (content-addressable store) for large blobs"
//! promises a weekly job that deletes blobs with `refcount = 0` and
//! `last_referenced > 30 days ago`, then `VACUUM`s the SQLite file.
//! This module implements that contract.
//!
//! Design notes:
//! - Per-batch transactions (256 rows at a time) keep `SQLITE_BUSY`
//!   from blocking ingestion for more than a couple ms at once.
//! - The runner asks SQLite for `freelist_count / page_count` after
//!   the deletes; if the ratio exceeds 25 % it issues `VACUUM` to
//!   reclaim disk.
//! - A safeguard refuses to delete when the candidate set covers
//!   more than 50 % of total blobs (catches an upstream refcount
//!   bug that returns 0 for everything). Operators override with
//!   `--force` after they've verified the cause.
//!
//! The library stays storage-bus-agnostic — the CLI emits the
//! `cortex.events.enriched` event of `kind=retention.cas_vacuum`.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use cortex_storage::cas::{CasError, CasStore, VacuumCandidate};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Vacuum runner inputs.
#[derive(Debug, Clone)]
pub struct VacuumOpts {
    /// Reference time (overridable via `--time-travel`).
    pub now: DateTime<Utc>,
    /// Minimum age in days before an unreferenced blob is eligible.
    /// Default 30 per spec 02.
    pub min_age_days: i64,
    /// Per-batch delete size (default 256).
    pub batch_size: u32,
    /// `true` skips the actual mutation. Counters still surface so
    /// the operator can preview what would happen.
    pub dry_run: bool,
    /// `true` overrides the catastrophic-deletion safeguard.
    pub force: bool,
    /// Free-pages ratio above which the runner issues `VACUUM`
    /// post-delete. Default `0.25` per spec.
    pub vacuum_ratio: f64,
}

impl VacuumOpts {
    /// Defaults per spec 02 — `now=Utc::now()`, `min_age_days=30`,
    /// `batch_size=256`, no dry-run, no force, vacuum_ratio=0.25.
    pub fn default_for(now: DateTime<Utc>) -> Self {
        Self {
            now,
            min_age_days: 30,
            batch_size: 256,
            dry_run: false,
            force: false,
            vacuum_ratio: 0.25,
        }
    }
}

/// Counters returned by [`run`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VacuumReport {
    /// Blobs actually deleted.
    pub blobs_dropped: u64,
    /// Sum of source `size` columns deleted (uncompressed bytes).
    pub bytes_reclaimed: u64,
    /// `true` when the catastrophic-deletion safeguard fired.
    pub safeguard_tripped: bool,
    /// Total blob count at start (used for the safeguard ratio).
    pub total_blobs: u64,
    /// `freelist_count / page_count` post-delete.
    pub free_pages_ratio: f64,
    /// `true` when the runner issued `VACUUM`.
    pub did_vacuum: bool,
    /// Wall-clock duration of the `VACUUM` call (0 when not run).
    pub vacuum_ms: u64,
}

/// Runner errors.
#[derive(Debug, Error)]
pub enum VacuumError {
    /// CAS store surfaced an error.
    #[error("cas: {0}")]
    Cas(#[from] CasError),
    /// Catastrophic-deletion safeguard fired without `--force`.
    #[error(
        "safeguard tripped: would_drop={would_drop} > 50% of total_blobs={total_blobs}; pass --force to override"
    )]
    SafeguardTripped {
        /// Number of blobs the run would have deleted.
        would_drop: u64,
        /// Total blobs in the store at start.
        total_blobs: u64,
    },
}

/// Run the vacuum against `store`.
pub fn run(store: &mut CasStore, opts: &VacuumOpts) -> Result<VacuumReport, VacuumError> {
    let mut report = VacuumReport::default();
    let cutoff = opts.now - Duration::days(opts.min_age_days);
    report.total_blobs = store.total_blob_count()?;

    // Prefetch every candidate so we can apply the safeguard before
    // any delete. This is a `LIMIT u32::MAX`-effective pull but the
    // CAS table is hash-indexed so the read is cheap; production
    // tables top out in the millions of rows.
    let candidates = store.select_vacuumable(cutoff, u32::MAX)?;
    let would_drop = candidates.len() as u64;

    // Catastrophic-deletion safeguard: refuse when we'd drop more
    // than half the store unless `--force`. Dry-run still surfaces
    // the trip flag in the report (so operators can preview the
    // problem) without erroring out — `--dry-run` is observe-only.
    let safeguard_would_trip =
        !opts.force && report.total_blobs > 0 && (would_drop * 2) > report.total_blobs;
    if safeguard_would_trip {
        report.safeguard_tripped = true;
        if !opts.dry_run {
            return Err(VacuumError::SafeguardTripped {
                would_drop,
                total_blobs: report.total_blobs,
            });
        }
    }

    if opts.dry_run {
        report.blobs_dropped = would_drop;
        report.bytes_reclaimed = candidates.iter().map(|c| c.size).sum();
        return Ok(report);
    }

    // Per-batch deletes inside `BEGIN IMMEDIATE` transactions.
    let batches = chunk_candidates(&candidates, opts.batch_size as usize);
    for batch in batches {
        let hashes: Vec<String> = batch.iter().map(|c| c.hash.clone()).collect();
        let dropped = store.delete_blobs(&hashes)?;
        report.blobs_dropped += dropped;
        report.bytes_reclaimed += batch.iter().map(|c| c.size).sum::<u64>();
    }

    // Decide on `VACUUM` based on free-pages ratio.
    let (freelist, page_count) = sqlite_page_stats(store)?;
    report.free_pages_ratio = if page_count == 0 {
        0.0
    } else {
        freelist as f64 / page_count as f64
    };
    if report.free_pages_ratio > opts.vacuum_ratio {
        let started = std::time::Instant::now();
        store
            .conn_mut()
            .execute_batch("VACUUM")
            .map_err(CasError::from)?;
        report.did_vacuum = true;
        report.vacuum_ms = started.elapsed().as_millis() as u64;
    }

    Ok(report)
}

fn chunk_candidates(candidates: &[VacuumCandidate], size: usize) -> Vec<&[VacuumCandidate]> {
    if size == 0 {
        return vec![candidates];
    }
    candidates.chunks(size).collect()
}

fn sqlite_page_stats(store: &CasStore) -> Result<(u64, u64), VacuumError> {
    let conn = store.conn();
    let freelist: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .map_err(CasError::from)?;
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .map_err(CasError::from)?;
    Ok((freelist.max(0) as u64, page_count.max(0) as u64))
}

/// Phase9c — refcount audit. Recomputes the expected refcount for
/// every hash by scanning a caller-supplied iterator of "external
/// references" (Vectorizer / Nexus / Meili payloads) and compares
/// the result against the stored `refcount`. Returns one row per
/// drift.
pub fn audit_refcounts<I>(
    store: &CasStore,
    references: I,
) -> Result<Vec<RefcountDrift>, VacuumError>
where
    I: IntoIterator<Item = String>,
{
    use std::collections::BTreeMap;
    let mut observed: BTreeMap<String, u64> = BTreeMap::new();
    for hash in references {
        *observed.entry(hash).or_insert(0) += 1;
    }
    // Pull every row's claimed refcount.
    let conn = store.conn();
    let mut stmt = conn
        .prepare("SELECT hash, refcount FROM cas_blobs")
        .map_err(CasError::from)?;
    let rows: Result<Vec<(String, u64)>, _> = stmt
        .query_map([], |row| {
            let h: String = row.get(0)?;
            let c: i64 = row.get(1)?;
            Ok((h, c.max(0) as u64))
        })
        .map_err(CasError::from)?
        .collect();
    let claimed: Vec<(String, u64)> = rows.map_err(CasError::from)?;

    let mut out: Vec<RefcountDrift> = Vec::new();
    let claimed_map: BTreeMap<String, u64> = claimed.into_iter().collect();
    let mut all_hashes: std::collections::BTreeSet<String> = observed.keys().cloned().collect();
    all_hashes.extend(claimed_map.keys().cloned());
    for hash in all_hashes {
        let c = claimed_map.get(&hash).copied().unwrap_or(0);
        let o = observed.get(&hash).copied().unwrap_or(0);
        if c != o {
            out.push(RefcountDrift {
                hash,
                claimed: c,
                observed: o,
            });
        }
    }
    Ok(out)
}

/// One drift row from [`audit_refcounts`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefcountDrift {
    /// Prefixed hash.
    pub hash: String,
    /// Refcount stored in `cas_blobs`.
    pub claimed: u64,
    /// Refcount the audit recomputed from external references.
    pub observed: u64,
}

/// Phase9c — apply the audit's findings to the store, correcting
/// every drifted row in one transaction. Returns the number of
/// rows updated.
pub fn fix_refcounts(store: &mut CasStore, drift: &[RefcountDrift]) -> Result<u64, VacuumError> {
    if drift.is_empty() {
        return Ok(0);
    }
    let tx = store
        .conn_mut()
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(CasError::from)?;
    let mut total: u64 = 0;
    {
        let mut stmt = tx
            .prepare("UPDATE cas_blobs SET refcount = ?1 WHERE hash = ?2")
            .map_err(CasError::from)?;
        for d in drift {
            total += stmt
                .execute(rusqlite::params![d.observed as i64, d.hash])
                .map_err(CasError::from)? as u64;
        }
    }
    tx.commit().map_err(CasError::from)?;
    Ok(total)
}

/// Convenience: open the metadata DB at `path` as a `CasStore`.
/// The CLI uses this to surface a friendlier error when the path
/// is wrong before delegating to [`run`].
pub fn open_store(path: &Path) -> Result<CasStore, CasError> {
    CasStore::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_storage::cas::CasContentType;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-29T18:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn put_aged_blob(
        store: &CasStore,
        body: &[u8],
        age_days: i64,
        ref_now: DateTime<Utc>,
    ) -> String {
        let hash = store.put(body, CasContentType::Text).unwrap();
        // Force last_referenced into the past via raw UPDATE — the
        // public `put`/`retain` helpers always stamp Utc::now().
        let target = ref_now - Duration::days(age_days);
        store
            .conn()
            .execute(
                "UPDATE cas_blobs SET last_referenced = ?1 WHERE hash = ?2",
                rusqlite::params![target.to_rfc3339(), hash],
            )
            .unwrap();
        hash
    }

    #[test]
    fn opts_default_uses_thirty_day_cutoff() {
        let opts = VacuumOpts::default_for(now());
        assert_eq!(opts.min_age_days, 30);
        assert_eq!(opts.batch_size, 256);
        assert!(!opts.dry_run);
        assert!(!opts.force);
        assert!((opts.vacuum_ratio - 0.25).abs() < 1e-9);
    }

    #[test]
    fn orphan_blob_older_than_thirty_days_is_deleted() {
        let mut store = CasStore::open_in_memory().unwrap();
        let stale = put_aged_blob(&store, b"orphan-old", 31, now());
        // Fresh blob — under the cutoff; must NOT be deleted.
        let _fresh = put_aged_blob(&store, b"orphan-fresh", 5, now());

        let opts = VacuumOpts::default_for(now());
        let report = run(&mut store, &opts).unwrap();
        assert_eq!(report.blobs_dropped, 1);
        assert!(report.bytes_reclaimed >= b"orphan-old".len() as u64);
        assert!(!store.contains(&stale).unwrap());
    }

    #[test]
    fn referenced_blob_is_preserved_even_when_old() {
        let mut store = CasStore::open_in_memory().unwrap();
        let referenced = put_aged_blob(&store, b"keep-me", 60, now());
        // Bump refcount to 2.
        store.retain(&referenced).unwrap();
        store.retain(&referenced).unwrap();
        // Re-age the row since `retain` stamps last_referenced.
        store
            .conn()
            .execute(
                "UPDATE cas_blobs SET last_referenced = ?1 WHERE hash = ?2",
                rusqlite::params![(now() - Duration::days(60)).to_rfc3339(), referenced],
            )
            .unwrap();
        let opts = VacuumOpts::default_for(now());
        let report = run(&mut store, &opts).unwrap();
        assert_eq!(report.blobs_dropped, 0);
        assert!(store.contains(&referenced).unwrap());
    }

    #[test]
    fn dry_run_records_counts_but_does_not_delete() {
        let mut store = CasStore::open_in_memory().unwrap();
        let stale = put_aged_blob(&store, b"old", 31, now());
        let mut opts = VacuumOpts::default_for(now());
        opts.dry_run = true;
        let report = run(&mut store, &opts).unwrap();
        assert_eq!(report.blobs_dropped, 1);
        // Source untouched.
        assert!(store.contains(&stale).unwrap());
    }

    #[test]
    fn safeguard_refuses_when_more_than_half_would_drop() {
        let mut store = CasStore::open_in_memory().unwrap();
        // 5 stale + 1 fresh — stale = 5/6 ≈ 83% of total, above 50%.
        for i in 0..5 {
            put_aged_blob(&store, format!("old-{i}").as_bytes(), 31, now());
        }
        put_aged_blob(&store, b"fresh", 5, now());
        let opts = VacuumOpts::default_for(now());
        match run(&mut store, &opts) {
            Err(VacuumError::SafeguardTripped {
                would_drop,
                total_blobs,
            }) => {
                assert_eq!(would_drop, 5);
                assert_eq!(total_blobs, 6);
            }
            other => panic!("expected SafeguardTripped, got {other:?}"),
        }
        // No deletes happened.
        assert_eq!(store.total_blob_count().unwrap(), 6);
    }

    #[test]
    fn safeguard_overridable_by_force() {
        let mut store = CasStore::open_in_memory().unwrap();
        for i in 0..5 {
            put_aged_blob(&store, format!("old-{i}").as_bytes(), 31, now());
        }
        put_aged_blob(&store, b"fresh", 5, now());
        let mut opts = VacuumOpts::default_for(now());
        opts.force = true;
        let report = run(&mut store, &opts).unwrap();
        assert_eq!(report.blobs_dropped, 5);
        assert_eq!(store.total_blob_count().unwrap(), 1);
    }

    #[test]
    fn safeguard_does_not_trip_on_empty_store() {
        let mut store = CasStore::open_in_memory().unwrap();
        let opts = VacuumOpts::default_for(now());
        let report = run(&mut store, &opts).unwrap();
        assert_eq!(report.blobs_dropped, 0);
        assert!(!report.safeguard_tripped);
    }

    #[test]
    fn audit_refcounts_reports_under_count_drift() {
        let store = CasStore::open_in_memory().unwrap();
        // Put a blob; retain once so refcount = 1.
        let hash = store.put(b"audit-me", CasContentType::Text).unwrap();
        store.retain(&hash).unwrap();
        // External references claim it appears 3 times.
        let drift =
            audit_refcounts(&store, vec![hash.clone(), hash.clone(), hash.clone()]).unwrap();
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].claimed, 1);
        assert_eq!(drift[0].observed, 3);
    }

    #[test]
    fn audit_refcounts_reports_over_count_drift() {
        let store = CasStore::open_in_memory().unwrap();
        let hash = store.put(b"audit-me", CasContentType::Text).unwrap();
        store.retain(&hash).unwrap();
        store.retain(&hash).unwrap();
        // No external references — should drift to 0.
        let drift = audit_refcounts(&store, std::iter::empty()).unwrap();
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].claimed, 2);
        assert_eq!(drift[0].observed, 0);
    }

    #[test]
    fn audit_refcounts_returns_empty_when_aligned() {
        let store = CasStore::open_in_memory().unwrap();
        let hash = store.put(b"aligned", CasContentType::Text).unwrap();
        store.retain(&hash).unwrap();
        let drift = audit_refcounts(&store, vec![hash.clone()]).unwrap();
        assert!(drift.is_empty());
    }

    #[test]
    fn fix_refcounts_writes_observed_to_store() {
        let mut store = CasStore::open_in_memory().unwrap();
        let hash = store.put(b"fix-me", CasContentType::Text).unwrap();
        store.retain(&hash).unwrap(); // refcount = 1
        let drift = vec![RefcountDrift {
            hash: hash.clone(),
            claimed: 1,
            observed: 4,
        }];
        let n = fix_refcounts(&mut store, &drift).unwrap();
        assert_eq!(n, 1);
        assert_eq!(store.refcount(&hash).unwrap(), 4);
    }

    #[test]
    fn fix_refcounts_no_op_on_empty_drift() {
        let mut store = CasStore::open_in_memory().unwrap();
        let n = fix_refcounts(&mut store, &[]).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn batches_split_candidates_evenly() {
        let cs: Vec<VacuumCandidate> = (0..513)
            .map(|i| VacuumCandidate {
                hash: format!("h{i}"),
                size: 1,
            })
            .collect();
        let batches = chunk_candidates(&cs, 256);
        // 513 = 256 + 256 + 1
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 256);
        assert_eq!(batches[1].len(), 256);
        assert_eq!(batches[2].len(), 1);
    }
}
