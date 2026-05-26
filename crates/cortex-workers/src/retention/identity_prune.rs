//! Phase14b §2 + §3 shared building blocks for hot- and cold-tier
//! identity-driven prune sweeps.
//!
//! Both sweeps walk the `event_identity` SQLite index (ADR-012),
//! resolve each row's age via the `archive_partition` path, and
//! cascade per-backend deletes when the row crosses the retention
//! cutoff. The cascade contract is:
//!
//! Hot-tier prune (90d) drops the per-backend rows from Meili plus
//! Nexus plus the per-kind Vectorizer FP32/PQ collections. The
//! parquet archive is preserved so the durable history stays intact
//! until cold-tier prune runs. Cold-tier prune (365d) drops every
//! backend — Meili, Nexus, archive partitions, and (via ADR-013's
//! collection-level re-encode) the cold-binary Vectorizer
//! collection.
//!
//! Both sweeps share [`IdentityCascadeOps`] and the
//! [`run_identity_cascade`] driver so the only divergence is which
//! backend legs each invokes per id.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use cortex_storage::identity::EventIdentity;
use regex::Regex;

/// Canonical archive partition path the writer stamps:
/// `events/year=YYYY/month=MM/day=DD/hour=HH/raw-NNNNN.parquet`. The
/// regex below pulls the four time fields so the pruner can derive
/// the partition's hour bucket without round-tripping through the
/// parquet header.
const PARTITION_RE: &str =
    r"year=(?P<year>\d{4})/month=(?P<month>\d{2})/day=(?P<day>\d{2})/hour=(?P<hour>\d{2})";

/// Parse the start-of-hour timestamp encoded by an archive partition
/// path. Returns `None` when the path does not match the canonical
/// layout — production callers MUST treat that as "skip, log warn"
/// rather than "prune".
pub fn partition_start_hour(path: &str) -> Option<DateTime<Utc>> {
    let re = match Regex::new(PARTITION_RE) {
        Ok(r) => r,
        Err(_) => return None,
    };
    let caps = re.captures(path)?;
    let year: i32 = caps.name("year")?.as_str().parse().ok()?;
    let month: u32 = caps.name("month")?.as_str().parse().ok()?;
    let day: u32 = caps.name("day")?.as_str().parse().ok()?;
    let hour: u32 = caps.name("hour")?.as_str().parse().ok()?;
    Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).single()
}

/// `true` when the partition hour bucket sits strictly before
/// `cutoff` — i.e. every envelope in the bucket is past the
/// retention horizon by at least the end-of-hour grace window. The
/// `+ 1h` safety margin guarantees an event with `occurred_at`
/// late in the hour still trips the cutoff cleanly.
pub fn partition_is_expired(path: &str, cutoff: DateTime<Utc>) -> bool {
    match partition_start_hour(path) {
        Some(start) => start + Duration::hours(1) <= cutoff,
        None => false,
    }
}

/// One expired identity row the sweep dispatches. The cascade ops
/// receive every populated native id so a per-backend delete only
/// fires when the projection actually landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredIdentity {
    /// Canonical envelope id — the cascade key.
    pub event_id: String,
    /// Nexus node_id, when the graph mapper stamped one.
    pub nexus_id: Option<String>,
    /// Vectorizer vector id, when the embedder stamped one.
    pub vec_id: Option<String>,
    /// Meili document id, when the fulltext indexer stamped one.
    pub meili_id: Option<String>,
    /// Archive partition path, when the writer stamped one.
    pub archive_partition: Option<String>,
}

impl ExpiredIdentity {
    /// Build from a full [`EventIdentity`] row.
    pub fn from_row(row: EventIdentity) -> Self {
        Self {
            event_id: row.event_id,
            nexus_id: row.nexus_id,
            vec_id: row.vec_id,
            meili_id: row.meili_id,
            archive_partition: row.archive_partition,
        }
    }
}

/// Source the sweep reads expired rows from. Production wires a
/// SQLite reader over `event_identity`; tests build a static list.
#[async_trait]
pub trait IdentitySource: Send + Sync {
    /// Return every `event_identity` row whose `archive_partition`
    /// hour bucket is at or before `cutoff`. The source SHOULD page
    /// internally — the driver consumes whatever the source returns
    /// in one go.
    async fn expired_identities(
        &self,
        cutoff: DateTime<Utc>,
    ) -> anyhow::Result<Vec<ExpiredIdentity>>;

    /// Drop the `event_identity` row for `event_id`. Called after
    /// every cascade leg succeeds so the doctor's post-prune
    /// assertion (`row absent ↔ event absent in every backend`)
    /// holds.
    async fn forget_identity(&self, event_id: &str) -> anyhow::Result<()>;
}

/// Per-backend cascade surface the sweeps invoke. Each leg is
/// idempotent — missing rows are silent ok, not errors.
#[async_trait]
pub trait IdentityCascadeOps: Send + Sync {
    /// Drop the Meili document at `meili_id` from the index that
    /// carries this event kind.
    async fn delete_meili(&self, meili_id: &str) -> anyhow::Result<()>;
    /// Drop the Nexus node at `nexus_id`.
    async fn delete_nexus(&self, nexus_id: &str) -> anyhow::Result<()>;
    /// Drop the per-event Vectorizer row from every hot collection
    /// the event could live in. Implementations SHOULD probe the
    /// full hot-collection set per ADR-012.
    async fn delete_vector(&self, vec_id: &str) -> anyhow::Result<()>;
    /// Rewrite `archive_partition` to drop the row carrying
    /// `event_id`. The parquet rewriter walks the file once per
    /// partition; the cascade groups expired ids by partition so
    /// the rewriter sees one batch per file.
    async fn drop_from_archive(
        &self,
        archive_partition: &str,
        event_id: &str,
    ) -> anyhow::Result<()>;
}

/// Which backend legs a cascade should hit. Hot-tier omits archive;
/// cold-tier hits every leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CascadePolicy {
    /// Cascade drops the Meili row when `meili_id.is_some()`.
    pub meili: bool,
    /// Cascade drops the Nexus node when `nexus_id.is_some()`.
    pub nexus: bool,
    /// Cascade drops the per-event Vectorizer row when
    /// `vec_id.is_some()`.
    pub vector: bool,
    /// Cascade rewrites the parquet partition when
    /// `archive_partition.is_some()`.
    pub archive: bool,
}

impl CascadePolicy {
    /// Hot-tier prune (90 d) — drops query-index rows but keeps the
    /// durable parquet archive intact.
    pub const HOT: Self = Self {
        meili: true,
        nexus: true,
        vector: true,
        archive: false,
    };

    /// Cold-tier prune (365 d) — drops every backend. The cold-tier
    /// Vectorizer rows are handled by the §4 collection-level
    /// re-encode, NOT this per-event cascade; this policy still
    /// flips `vector = true` so the cascade probes the hot
    /// collections for stragglers a previous hot-tier run missed.
    pub const COLD: Self = Self {
        meili: true,
        nexus: true,
        vector: true,
        archive: true,
    };
}

/// Per-event outcome surfaced in [`CascadeReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CascadeOutcome {
    /// Event id that was processed.
    pub event_id: String,
    /// `true` when every applicable backend leg succeeded.
    pub ok: bool,
    /// Reason captured when `ok == false`. Populated for the first
    /// failing leg; subsequent legs are skipped for that event.
    pub failure_reason: Option<String>,
}

/// Aggregate counters the driver hands back. The Sweep wrapper
/// folds these into `SweepReport::{rows_processed, ...}`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CascadeReport {
    /// Events the driver processed (sum of `ok` + `failed`).
    pub processed: u64,
    /// Events whose every applicable cascade leg succeeded AND the
    /// `event_identity` row was dropped.
    pub ok: u64,
    /// Events whose cascade hit an error in any leg. The
    /// `event_identity` row is left intact so the next sweep
    /// retries the cascade.
    pub failed: u64,
    /// Per-event outcomes — the driver returns them so the sweep
    /// IT can pin the exact id set that survived a failure path.
    pub outcomes: Vec<CascadeOutcome>,
}

/// Drive the cascade for every expired identity. Returns counters
/// the sweep folds into its `SweepReport`.
pub async fn run_identity_cascade(
    source: Arc<dyn IdentitySource>,
    ops: Arc<dyn IdentityCascadeOps>,
    policy: CascadePolicy,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<CascadeReport> {
    let expired = source.expired_identities(cutoff).await?;
    let mut report = CascadeReport::default();
    for row in expired {
        report.processed += 1;
        match cascade_one(ops.as_ref(), &row, policy).await {
            Ok(()) => {
                if let Err(err) = source.forget_identity(&row.event_id).await {
                    report.failed += 1;
                    report.outcomes.push(CascadeOutcome {
                        event_id: row.event_id.clone(),
                        ok: false,
                        failure_reason: Some(format!("forget_identity: {err}")),
                    });
                    continue;
                }
                report.ok += 1;
                report.outcomes.push(CascadeOutcome {
                    event_id: row.event_id,
                    ok: true,
                    failure_reason: None,
                });
            }
            Err(err) => {
                report.failed += 1;
                report.outcomes.push(CascadeOutcome {
                    event_id: row.event_id,
                    ok: false,
                    failure_reason: Some(err.to_string()),
                });
            }
        }
    }
    Ok(report)
}

async fn cascade_one(
    ops: &dyn IdentityCascadeOps,
    row: &ExpiredIdentity,
    policy: CascadePolicy,
) -> anyhow::Result<()> {
    if policy.meili {
        if let Some(id) = row.meili_id.as_deref() {
            ops.delete_meili(id).await?;
        }
    }
    if policy.nexus {
        if let Some(id) = row.nexus_id.as_deref() {
            ops.delete_nexus(id).await?;
        }
    }
    if policy.vector {
        if let Some(id) = row.vec_id.as_deref() {
            ops.delete_vector(id).await?;
        }
    }
    if policy.archive {
        if let Some(partition) = row.archive_partition.as_deref() {
            ops.drop_from_archive(partition, &row.event_id).await?;
        }
    }
    Ok(())
}

/// Group expired ids by archive partition. The cold-tier sweep
/// uses this to batch the parquet rewrite — one rewrite per
/// partition file rather than one per id.
pub fn group_by_partition(rows: &[ExpiredIdentity]) -> Vec<(String, Vec<String>)> {
    use std::collections::BTreeMap;
    let mut by_partition: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        if let Some(partition) = row.archive_partition.as_deref() {
            by_partition
                .entry(partition.to_string())
                .or_default()
                .push(row.event_id.clone());
        }
    }
    by_partition.into_iter().collect()
}

/// Render a one-line cron summary the dashboard renders. Kept here
/// (not on `CascadeReport`) so the sweep wrappers can compose it
/// with the per-sweep `SweepReport::message`.
pub fn render_cascade_summary(policy: CascadePolicy, report: &CascadeReport) -> String {
    let label = if policy.archive { "cold" } else { "hot" };
    format!(
        "{label}-tier cascade: processed={} ok={} failed={}",
        report.processed, report.ok, report.failed,
    )
}

// ---- In-memory fixtures ------------------------------------------

/// Static [`IdentitySource`] backed by an in-memory `Vec`. Lets the
/// sweeps' unit tests + the §3.4 100-event IT drive the cascade
/// deterministically without spinning up SQLite.
pub struct StaticIdentitySource {
    rows: tokio::sync::Mutex<Vec<EventIdentity>>,
    occurred_at_by_event: std::collections::BTreeMap<String, DateTime<Utc>>,
}

impl StaticIdentitySource {
    /// Build a source seeded with `rows`. Each `EventIdentity` MUST
    /// carry an `archive_partition` that parses via
    /// [`partition_start_hour`] so the expiry filter has a real
    /// occurred-at to compare against.
    pub fn new(rows: Vec<EventIdentity>) -> Self {
        let occurred_at_by_event = rows
            .iter()
            .filter_map(|r| {
                r.archive_partition
                    .as_deref()
                    .and_then(partition_start_hour)
                    .map(|t| (r.event_id.clone(), t))
            })
            .collect();
        Self {
            rows: tokio::sync::Mutex::new(rows),
            occurred_at_by_event,
        }
    }

    /// Snapshot the currently-known event_ids for assertions.
    pub async fn known_ids(&self) -> BTreeSet<String> {
        self.rows
            .lock()
            .await
            .iter()
            .map(|r| r.event_id.clone())
            .collect()
    }
}

#[async_trait]
impl IdentitySource for StaticIdentitySource {
    async fn expired_identities(
        &self,
        cutoff: DateTime<Utc>,
    ) -> anyhow::Result<Vec<ExpiredIdentity>> {
        let rows = self.rows.lock().await;
        Ok(rows
            .iter()
            .filter(|r| {
                self.occurred_at_by_event
                    .get(&r.event_id)
                    .map(|t| *t + Duration::hours(1) <= cutoff)
                    .unwrap_or(false)
            })
            .cloned()
            .map(ExpiredIdentity::from_row)
            .collect())
    }

    async fn forget_identity(&self, event_id: &str) -> anyhow::Result<()> {
        let mut rows = self.rows.lock().await;
        rows.retain(|r| r.event_id != event_id);
        Ok(())
    }
}

/// Recording [`IdentityCascadeOps`] for tests. Captures every leg
/// invocation so tests can assert "Meili called with X", "archive
/// rewrite for partition Y got id Z", etc. Variants of the fixture
/// can inject errors via [`Self::inject_meili_failure`] etc.
#[derive(Default)]
pub struct RecordingCascadeOps {
    state: tokio::sync::Mutex<RecordingState>,
}

#[derive(Default)]
struct RecordingState {
    meili: Vec<String>,
    nexus: Vec<String>,
    vector: Vec<String>,
    archive: Vec<(String, String)>,
    meili_failure: Option<String>,
}

impl RecordingCascadeOps {
    /// Fresh, empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the Meili ids the cascade called with.
    pub async fn meili_calls(&self) -> Vec<String> {
        self.state.lock().await.meili.clone()
    }

    /// Snapshot the Nexus ids the cascade called with.
    pub async fn nexus_calls(&self) -> Vec<String> {
        self.state.lock().await.nexus.clone()
    }

    /// Snapshot the Vectorizer ids the cascade called with.
    pub async fn vector_calls(&self) -> Vec<String> {
        self.state.lock().await.vector.clone()
    }

    /// Snapshot the (partition, event_id) archive calls.
    pub async fn archive_calls(&self) -> Vec<(String, String)> {
        self.state.lock().await.archive.clone()
    }

    /// Inject a one-shot Meili failure so the next cascade leg
    /// returns an error.
    pub async fn inject_meili_failure(&self, reason: impl Into<String>) {
        self.state.lock().await.meili_failure = Some(reason.into());
    }
}

#[async_trait]
impl IdentityCascadeOps for RecordingCascadeOps {
    async fn delete_meili(&self, meili_id: &str) -> anyhow::Result<()> {
        let mut s = self.state.lock().await;
        if let Some(reason) = s.meili_failure.take() {
            return Err(anyhow::anyhow!(reason));
        }
        s.meili.push(meili_id.to_string());
        Ok(())
    }
    async fn delete_nexus(&self, nexus_id: &str) -> anyhow::Result<()> {
        self.state.lock().await.nexus.push(nexus_id.to_string());
        Ok(())
    }
    async fn delete_vector(&self, vec_id: &str) -> anyhow::Result<()> {
        self.state.lock().await.vector.push(vec_id.to_string());
        Ok(())
    }
    async fn drop_from_archive(
        &self,
        archive_partition: &str,
        event_id: &str,
    ) -> anyhow::Result<()> {
        self.state
            .lock()
            .await
            .archive
            .push((archive_partition.to_string(), event_id.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(event: &str, partition: &str) -> EventIdentity {
        EventIdentity {
            event_id: event.into(),
            nexus_id: Some(format!("nxs-{event}")),
            vec_id: Some(format!("vec-{event}")),
            meili_id: Some(format!("mli-{event}")),
            archive_partition: Some(partition.into()),
        }
    }

    fn ts(rfc: &str) -> DateTime<Utc> {
        rfc.parse().expect("rfc3339")
    }

    #[test]
    fn partition_start_hour_parses_canonical_path() {
        use chrono::{Datelike, Timelike};
        let p = "events/year=2026/month=05/day=24/hour=10/raw-00000.parquet";
        let t = partition_start_hour(p).expect("parse");
        assert_eq!(t.year(), 2026);
        assert_eq!(t.month(), 5);
        assert_eq!(t.day(), 24);
        assert_eq!(t.hour(), 10);
        assert_eq!(t.minute(), 0);
    }

    #[test]
    fn partition_start_hour_returns_none_on_bad_path() {
        assert!(partition_start_hour("events/garbage").is_none());
        assert!(partition_start_hour("year=2026/month=05/day=24").is_none());
    }

    #[test]
    fn partition_is_expired_respects_end_of_hour_grace() {
        let p = "events/year=2026/month=05/day=24/hour=10/raw-00000.parquet";
        // Cutoff one nanosecond past the end of hour 10 → expired.
        assert!(partition_is_expired(p, ts("2026-05-24T11:00:01Z")));
        // Cutoff at the end of hour 10 → expired (boundary inclusive).
        assert!(partition_is_expired(p, ts("2026-05-24T11:00:00Z")));
        // Cutoff one second before end of hour 10 → not expired.
        assert!(!partition_is_expired(p, ts("2026-05-24T10:59:59Z")));
    }

    #[tokio::test]
    async fn static_source_filters_by_cutoff() {
        let source = StaticIdentitySource::new(vec![
            row(
                "e-old",
                "events/year=2024/month=01/day=01/hour=00/raw.parquet",
            ),
            row(
                "e-new",
                "events/year=2026/month=05/day=20/hour=00/raw.parquet",
            ),
        ]);
        let expired = source
            .expired_identities(ts("2026-04-01T00:00:00Z"))
            .await
            .unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].event_id, "e-old");
    }

    #[tokio::test]
    async fn cascade_hot_drops_meili_nexus_vector_keeps_archive_intact() {
        let source = Arc::new(StaticIdentitySource::new(vec![row(
            "e-1",
            "events/year=2024/month=01/day=01/hour=00/raw.parquet",
        )]));
        let ops = Arc::new(RecordingCascadeOps::new());
        let report = run_identity_cascade(
            source.clone(),
            ops.clone(),
            CascadePolicy::HOT,
            ts("2026-04-01T00:00:00Z"),
        )
        .await
        .unwrap();

        assert_eq!(report.processed, 1);
        assert_eq!(report.ok, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(ops.meili_calls().await, vec!["mli-e-1"]);
        assert_eq!(ops.nexus_calls().await, vec!["nxs-e-1"]);
        assert_eq!(ops.vector_calls().await, vec!["vec-e-1"]);
        assert!(ops.archive_calls().await.is_empty(), "hot keeps archive");
        // Forgotten from the source.
        assert!(!source.known_ids().await.contains("e-1"));
    }

    #[tokio::test]
    async fn cascade_cold_drops_all_backends_including_archive() {
        let source = Arc::new(StaticIdentitySource::new(vec![row(
            "e-c",
            "events/year=2024/month=01/day=01/hour=00/raw.parquet",
        )]));
        let ops = Arc::new(RecordingCascadeOps::new());
        let report = run_identity_cascade(
            source,
            ops.clone(),
            CascadePolicy::COLD,
            ts("2026-04-01T00:00:00Z"),
        )
        .await
        .unwrap();

        assert_eq!(report.ok, 1);
        assert_eq!(
            ops.archive_calls().await,
            vec![(
                "events/year=2024/month=01/day=01/hour=00/raw.parquet".to_string(),
                "e-c".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn cascade_failure_leaves_identity_row_alive_for_retry() {
        let source = Arc::new(StaticIdentitySource::new(vec![row(
            "e-fail",
            "events/year=2024/month=01/day=01/hour=00/raw.parquet",
        )]));
        let ops = Arc::new(RecordingCascadeOps::new());
        ops.inject_meili_failure("synthetic meili").await;
        let report = run_identity_cascade(
            source.clone(),
            ops.clone(),
            CascadePolicy::HOT,
            ts("2026-04-01T00:00:00Z"),
        )
        .await
        .unwrap();

        assert_eq!(report.failed, 1);
        assert_eq!(report.ok, 0);
        assert!(report.outcomes[0]
            .failure_reason
            .as_deref()
            .unwrap_or("")
            .contains("synthetic meili"));
        // Identity row preserved — next sweep retries the cascade.
        assert!(source.known_ids().await.contains("e-fail"));
        // Downstream legs (nexus, vector) NOT invoked because the
        // cascade short-circuits on the first leg failure.
        assert!(ops.nexus_calls().await.is_empty());
        assert!(ops.vector_calls().await.is_empty());
    }

    #[test]
    fn group_by_partition_collapses_ids_per_file() {
        let rows = vec![
            ExpiredIdentity {
                event_id: "a".into(),
                nexus_id: None,
                vec_id: None,
                meili_id: None,
                archive_partition: Some("p1".into()),
            },
            ExpiredIdentity {
                event_id: "b".into(),
                nexus_id: None,
                vec_id: None,
                meili_id: None,
                archive_partition: Some("p1".into()),
            },
            ExpiredIdentity {
                event_id: "c".into(),
                nexus_id: None,
                vec_id: None,
                meili_id: None,
                archive_partition: Some("p2".into()),
            },
        ];
        let grouped = group_by_partition(&rows);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].0, "p1");
        assert_eq!(grouped[0].1, vec!["a", "b"]);
        assert_eq!(grouped[1].0, "p2");
        assert_eq!(grouped[1].1, vec!["c"]);
    }

    #[test]
    fn render_summary_uses_hot_or_cold_label_per_policy() {
        let r = CascadeReport {
            processed: 5,
            ok: 4,
            failed: 1,
            outcomes: vec![],
        };
        assert!(render_cascade_summary(CascadePolicy::HOT, &r).starts_with("hot-tier"));
        assert!(render_cascade_summary(CascadePolicy::COLD, &r).starts_with("cold-tier"));
    }
}
