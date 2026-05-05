//! Phase11o — Consolidation pruner.
//!
//! Walks the `cortex_consolidations` Meili index for every active
//! consolidation, resolves its `source_event_id` to the owning
//! Vectorizer collection, and demotes vectors between tiers per the
//! 0–7 d / 7–90 d / 90–365 d / >365 d schedule:
//!
//! - hot  (`cortex.consolidation.fp32`)  — 0..7  d
//! - warm (`cortex.consolidation.pq`)    — 7..90 d
//! - cold (`cortex.cold.binary`)         — 90..365 d
//! - expired                             — >365 d, hard purge
//!
//! The pruner runs nightly via the retention daemon (cron name
//! `retention.consolidation_prune`, default `0 3 * * *`). Per-tier
//! action plans go through three sinks defined in this module:
//!
//! - [`vectorizer_sink`] — calls
//!   [`crate::embedder::vectorizer_client::VectorizerClient::move_vectors`]
//!   for warm/cold transitions, batched at
//!   [`MOVE_BATCH_SIZE`] ids per request.
//! - [`meili_sink`] — drops the high-cost fields (`body`, `summary`,
//!   `outcome_distribution`) on cold-tier rows so the keyword lane
//!   keeps a minimal record while the disk footprint stays bounded.
//! - [`purge`] — hard-purge sink callable from the
//!   `/cortex forget <event_id>` MCP tool. Cascades to Vectorizer,
//!   Meili, Nexus, and the Parquet archive.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use chrono::{DateTime, Utc};
use cortex_storage::names::{
    COLLECTION_COLD_BINARY, COLLECTION_CONSOLIDATION_FP32, COLLECTION_CONSOLIDATION_PQ,
};

pub mod engine;
pub mod meili_sink;
pub mod purge;
pub mod vectorizer_sink;

/// Phase11o §2.2 — chunk size for `VectorizerClient::move_vectors`
/// batches. Bounded so a single transient 5xx never loses more than
/// a day's worth of demotions.
pub const MOVE_BATCH_SIZE: usize = 256;

/// Tier classification for a consolidation's source event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneTier {
    /// 0..7 d — vectors live in `cortex.consolidation.fp32`.
    Hot,
    /// 7..90 d — vectors live in `cortex.consolidation.pq`.
    Warm,
    /// 90..365 d — vectors live in `cortex.cold.binary`; the Meili
    /// row is stripped of `body`, `summary`, `outcome_distribution`.
    Cold,
    /// >365 d — eligible for hard purge across every backend.
    Expired,
}

impl PruneTier {
    /// Bucket an age (in whole days) into the matching tier.
    pub fn from_age_days(age_days: i64) -> Self {
        match age_days {
            d if d < 7 => PruneTier::Hot,
            d if d < 90 => PruneTier::Warm,
            d if d < 365 => PruneTier::Cold,
            _ => PruneTier::Expired,
        }
    }

    /// Owning Vectorizer collection for this tier — the address the
    /// pruner reads from when planning a demotion. Returns `None`
    /// for [`PruneTier::Expired`] (no live collection).
    pub fn vectorizer_collection(self) -> Option<&'static str> {
        match self {
            PruneTier::Hot => Some(COLLECTION_CONSOLIDATION_FP32),
            PruneTier::Warm => Some(COLLECTION_CONSOLIDATION_PQ),
            PruneTier::Cold => Some(COLLECTION_COLD_BINARY),
            PruneTier::Expired => None,
        }
    }
}

/// One concrete demotion the pruner intends to execute. Produced by
/// [`plan_demotion`] and consumed by [`vectorizer_sink::demote`] /
/// [`meili_sink::demote`] / [`purge::forget`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemotionAction {
    /// `event_id` of the consolidation owning the source events.
    pub consolidation_id: String,
    /// Tier the source event currently lives in.
    pub from: PruneTier,
    /// Tier it is being demoted to.
    pub to: PruneTier,
    /// Vector ids inside the source collection that should move.
    /// Empty list ⇒ Meili-only demotion (cold-tier field stripping
    /// without a vector move; happens when the vector already lives
    /// in `cortex.cold.binary`).
    pub vector_ids: Vec<String>,
}

impl DemotionAction {
    /// Source Vectorizer collection name. Returns `None` when the
    /// consolidation's source vectors have already expired and there
    /// is nothing to read from.
    pub fn src_collection(&self) -> Option<&'static str> {
        self.from.vectorizer_collection()
    }

    /// Destination collection name, mirroring [`Self::src_collection`].
    pub fn dst_collection(&self) -> Option<&'static str> {
        self.to.vectorizer_collection()
    }
}

/// Summary returned by a pruner run. Surfaced in the `/v1/health`
/// `pruner` block (§3.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Total consolidations the pruner inspected this run.
    pub consolidations_seen: u64,
    /// Vectors successfully moved per `(from, to)` tier pair. Keys
    /// are formatted `"{from}->{to}"` (e.g. `"hot->warm"`) so the
    /// health JSON keeps a stable wire shape.
    pub events_demoted_per_tier: std::collections::BTreeMap<String, u64>,
    /// Vectors hard-purged this run (`Expired` tier).
    pub events_purged: u64,
    /// Wall-clock duration of the run.
    pub last_run_duration_ms: u64,
    /// Last error message, populated when the run aborted before
    /// completion. `None` on a clean run, even when individual
    /// per-id failures occurred (those are counted in `events_failed`).
    pub last_error: Option<String>,
    /// Total per-id failures across every batch. Distinct from
    /// `last_error`, which only fires on a fatal error that stopped
    /// the whole run.
    pub events_failed: u64,
}

/// Plan a single demotion given the consolidation's age + the vector
/// ids the caller resolved from Meili. Returns `None` when the
/// consolidation is still in the hot tier (no demotion to perform).
pub fn plan_demotion(
    consolidation_id: impl Into<String>,
    occurred_at: DateTime<Utc>,
    now: DateTime<Utc>,
    vector_ids: Vec<String>,
) -> Option<DemotionAction> {
    let age_days = (now - occurred_at).num_days().max(0);
    let current = PruneTier::from_age_days(age_days);
    if current == PruneTier::Hot {
        return None;
    }
    // The "from" tier is the one *immediately above* the current
    // bucket: a Warm consolidation came from Hot; Cold from Warm;
    // Expired from Cold. The pruner operates on the next-step
    // demotion, not the absolute target — so a Warm consolidation
    // moves Hot→Warm at the 7 d boundary, then Warm→Cold at 90 d on
    // a later run.
    let (from, to) = match current {
        PruneTier::Warm => (PruneTier::Hot, PruneTier::Warm),
        PruneTier::Cold => (PruneTier::Warm, PruneTier::Cold),
        PruneTier::Expired => (PruneTier::Cold, PruneTier::Expired),
        PruneTier::Hot => return None,
    };
    Some(DemotionAction {
        consolidation_id: consolidation_id.into(),
        from,
        to,
        vector_ids,
    })
}

/// Format a tier-pair as a stable key for [`PruneReport`] /
/// `/v1/health` JSON.
pub fn tier_pair_key(from: PruneTier, to: PruneTier) -> String {
    fn label(t: PruneTier) -> &'static str {
        match t {
            PruneTier::Hot => "hot",
            PruneTier::Warm => "warm",
            PruneTier::Cold => "cold",
            PruneTier::Expired => "expired",
        }
    }
    format!("{}->{}", label(from), label(to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    #[test]
    fn tier_buckets_match_spec() {
        assert_eq!(PruneTier::from_age_days(0), PruneTier::Hot);
        assert_eq!(PruneTier::from_age_days(6), PruneTier::Hot);
        assert_eq!(PruneTier::from_age_days(7), PruneTier::Warm);
        assert_eq!(PruneTier::from_age_days(89), PruneTier::Warm);
        assert_eq!(PruneTier::from_age_days(90), PruneTier::Cold);
        assert_eq!(PruneTier::from_age_days(364), PruneTier::Cold);
        assert_eq!(PruneTier::from_age_days(365), PruneTier::Expired);
        assert_eq!(PruneTier::from_age_days(10_000), PruneTier::Expired);
    }

    #[test]
    fn collection_mapping_matches_storage_names() {
        assert_eq!(
            PruneTier::Hot.vectorizer_collection(),
            Some(COLLECTION_CONSOLIDATION_FP32)
        );
        assert_eq!(
            PruneTier::Warm.vectorizer_collection(),
            Some(COLLECTION_CONSOLIDATION_PQ)
        );
        assert_eq!(
            PruneTier::Cold.vectorizer_collection(),
            Some(COLLECTION_COLD_BINARY)
        );
        assert_eq!(PruneTier::Expired.vectorizer_collection(), None);
    }

    #[test]
    fn hot_consolidations_yield_no_action() {
        let now = ts(2026, 5, 4);
        let occurred = ts(2026, 5, 1); // 3 days old
        let action = plan_demotion("c1", occurred, now, vec!["v1".into()]);
        assert!(action.is_none());
    }

    #[test]
    fn warm_consolidations_demote_hot_to_warm() {
        let now = ts(2026, 5, 4);
        let occurred = ts(2026, 4, 25); // 9 days old
        let action = plan_demotion("c2", occurred, now, vec!["v1".into(), "v2".into()]).unwrap();
        assert_eq!(action.from, PruneTier::Hot);
        assert_eq!(action.to, PruneTier::Warm);
        assert_eq!(
            action.src_collection(),
            Some(COLLECTION_CONSOLIDATION_FP32)
        );
        assert_eq!(action.dst_collection(), Some(COLLECTION_CONSOLIDATION_PQ));
    }

    #[test]
    fn cold_consolidations_demote_warm_to_cold() {
        let now = ts(2026, 5, 4);
        let occurred = ts(2026, 1, 1); // 123 days old
        let action = plan_demotion("c3", occurred, now, vec!["v1".into()]).unwrap();
        assert_eq!(action.from, PruneTier::Warm);
        assert_eq!(action.to, PruneTier::Cold);
        assert_eq!(action.dst_collection(), Some(COLLECTION_COLD_BINARY));
    }

    #[test]
    fn expired_consolidations_route_to_purge() {
        let now = ts(2026, 5, 4);
        let occurred = ts(2025, 1, 1); // 488 days old
        let action = plan_demotion("c4", occurred, now, vec!["v1".into()]).unwrap();
        assert_eq!(action.from, PruneTier::Cold);
        assert_eq!(action.to, PruneTier::Expired);
        assert_eq!(action.dst_collection(), None);
    }

    #[test]
    fn tier_pair_keys_are_stable() {
        assert_eq!(tier_pair_key(PruneTier::Hot, PruneTier::Warm), "hot->warm");
        assert_eq!(
            tier_pair_key(PruneTier::Cold, PruneTier::Expired),
            "cold->expired"
        );
    }
}
