//! Phase27c §3 — community-aware graph entity dedup.
//!
//! Graph entity resolution: find node pairs that are the same
//! real-world entity ("User" the auth struct vs "User" the shipping
//! struct are NOT; "nexus_client" and "NexusClient" in the same
//! subsystem probably ARE) and plan their merge. The pipeline, per
//! the phase27c proposal:
//!
//! 1. **Entropy gate** — degenerate names ("x", "aaa", "id") carry
//!    too little signal to dedup safely; skip them.
//! 2. **Blocking** — hashed-4-gram cosine (`cortex_core::textsim`,
//!    the §3.1 shared lift) inside per-label buckets. (The proposal
//!    said "MinHash"; no MinHash existed anywhere in the workspace —
//!    verified — and the lifted n-gram bag plays the same cheap
//!    approximate-blocking role. Swap in LSH banding when bucket
//!    sizes make O(n²) cosine hurt.)
//! 3. **Jaro-Winkler verify** (`strsim`) on the candidate pairs.
//! 4. **Same-community signal** — equal `community_id` boosts the
//!    verify score, different communities penalise it. This is the
//!    graphify homonym separator: "User"@auth vs "User"@shipping
//!    land in different communities and the penalty keeps them
//!    apart without embeddings.
//! 5. **Union-find merge** with survivor-id preference (smallest id
//!    wins — deterministic across runs).
//!
//! An optional LLM tiebreaker (§3.3) is consulted ONLY for pairs in
//! the ambiguous score band, behind a config flag that defaults OFF;
//! the [`TieBreaker`] trait keeps the model call out of this module
//! so the pass stays fully offline-testable.
//!
//! This module produces a **merge plan** ([`MergeGroup`]s), not
//! writes: applying a merge rewires edges in Nexus, and the worker
//! that would run this (the phase27b §2.5 community worker) is gated
//! on the semantic projection (ADR-027). Plan generation is complete
//! and tested now; application rides the same unblock.

use std::collections::HashMap;

use async_trait::async_trait;
use cortex_core::textsim::{cosine, ngram_vector, shannon_entropy_bits};

/// Tunables for the dedup pass.
#[derive(Debug, Clone)]
pub struct DedupConfig {
    /// Names below this Shannon entropy (bits) are skipped by the
    /// gate. `render_edge_merge` ≈ 2.55 bits; `aaaa` = 0.
    pub entropy_min_bits: f64,
    /// Blocking: candidate pairs must clear this n-gram cosine.
    pub block_threshold: f32,
    /// Verify: pairs at or above this adjusted Jaro-Winkler merge.
    pub verify_threshold: f64,
    /// Added to the Jaro-Winkler score when both nodes carry the
    /// same `community_id`.
    pub community_boost: f64,
    /// Subtracted when both carry a `community_id` and they differ
    /// (the homonym separator).
    pub community_penalty: f64,
    /// Width of the ambiguous band below `verify_threshold` in which
    /// the optional tiebreaker is consulted (when enabled).
    pub ambiguous_band: f64,
    /// §3.3 flag — consult the [`TieBreaker`] for ambiguous pairs.
    /// Defaults OFF: the deterministic pipeline decides alone.
    pub llm_tiebreaker_enabled: bool,
    /// n-gram vector dimensionality for blocking.
    pub ngram_dim: usize,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            entropy_min_bits: 1.5,
            // 0.50 keeps snake_case/camelCase variants of the same
            // name inside the block: an underscore shifts every
            // following 4-gram window, so "nexus_client" vs
            // "nexusclient" only reaches ~0.59 cosine.
            block_threshold: 0.50,
            verify_threshold: 0.90,
            community_boost: 0.05,
            // Big enough that an EXACT homonym (JW = 1.0) in a
            // different community drops below verify_threshold into
            // the ambiguous band: 1.0 - 0.15 = 0.85 < 0.90.
            community_penalty: 0.15,
            ambiguous_band: 0.07,
            llm_tiebreaker_enabled: false,
            ngram_dim: 256,
        }
    }
}

/// One graph entity under consideration.
#[derive(Debug, Clone)]
pub struct EntityRecord {
    /// Node id (`_id`).
    pub id: String,
    /// Node label — pairs never cross labels (a `Symbol` never
    /// merges with an `Artifact`).
    pub label: String,
    /// Display name the similarity runs on.
    pub name: String,
    /// Community membership from the phase27b writeback, when the
    /// partition has run (`None` until ADR-027 unblocks it).
    pub community_id: Option<u32>,
}

/// One planned merge: every node in `merged` folds into `survivor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeGroup {
    /// Surviving node id (smallest id in the group).
    pub survivor: String,
    /// Ids merging into the survivor, sorted.
    pub merged: Vec<String>,
}

/// §3.3 — tiebreaker consulted for ambiguous pairs when
/// `llm_tiebreaker_enabled` is set. Implementations wrap the shared
/// summariser stack (Claude CLI) + the consolidator's cost budget;
/// this trait keeps the pass offline-testable.
#[async_trait]
pub trait TieBreaker: Send + Sync {
    /// `true` when `a` and `b` name the same entity.
    async fn same_entity(&self, a: &EntityRecord, b: &EntityRecord) -> bool;
}

/// Default tiebreaker: never merges an ambiguous pair. Used when the
/// flag is off (and as the conservative fallback when a live
/// tiebreaker errors upstream).
pub struct DisabledTieBreaker;

#[async_trait]
impl TieBreaker for DisabledTieBreaker {
    async fn same_entity(&self, _a: &EntityRecord, _b: &EntityRecord) -> bool {
        false
    }
}

/// §3.3 — live tiebreaker: one budget-gated yes/no call through the
/// shared [`Summariser`] stack (Claude CLI in production — see
/// `consolidator/summariser_cli.rs`). Every answer that isn't an
/// unambiguous "yes" — parse failure, summariser error, budget
/// ceiling — resolves to `false`: a missed merge is recoverable on
/// the next pass, a wrong merge corrupts the graph.
pub struct SummariserTieBreaker {
    summariser: std::sync::Arc<dyn crate::consolidator::summariser::Summariser>,
    ledger: std::sync::Arc<std::sync::Mutex<crate::consolidator::cost_telemetry::CostLedger>>,
    budget: crate::consolidator::cost_telemetry::CostBudget,
    /// Per-call cost estimate used for the affordability pre-check.
    est_cents: u32,
}

/// Ledger label the tiebreaker records its spend under.
pub const DEDUP_TIEBREAKER_GRAIN_LABEL: &str = "dedup_tiebreaker";

impl SummariserTieBreaker {
    /// Build a tiebreaker over `summariser`, recording spend into
    /// `ledger` and refusing calls that would breach `budget`.
    pub fn new(
        summariser: std::sync::Arc<dyn crate::consolidator::summariser::Summariser>,
        ledger: std::sync::Arc<std::sync::Mutex<crate::consolidator::cost_telemetry::CostLedger>>,
        budget: crate::consolidator::cost_telemetry::CostBudget,
    ) -> Self {
        Self {
            summariser,
            ledger,
            budget,
            est_cents: 5,
        }
    }
}

#[async_trait]
impl TieBreaker for SummariserTieBreaker {
    async fn same_entity(&self, a: &EntityRecord, b: &EntityRecord) -> bool {
        // Budget gate BEFORE the call — same contract as the
        // consolidator orchestrator's gate_budget.
        {
            let ledger = match self.ledger.lock() {
                Ok(l) => l,
                Err(_) => return false,
            };
            if !self.budget.can_afford(&ledger, self.est_cents) {
                tracing::warn!(
                    a = %a.id, b = %b.id,
                    "dedup tiebreaker skipped: daily cost budget exhausted"
                );
                return false;
            }
        }
        let prompt = format!(
            "Two entities were extracted from the same codebase's graph.\n\
             Entity A: label={} name=\"{}\" community={:?}\n\
             Entity B: label={} name=\"{}\" community={:?}\n\
             Do these two names refer to the SAME code entity (the same \
             function/struct/module), as opposed to two different entities \
             that happen to have similar names? Answer with exactly one \
             word: yes or no.",
            a.label, a.name, a.community_id, b.label, b.name, b.community_id
        );
        let result = self
            .summariser
            .summarise(crate::consolidator::summariser::SummariserRequest {
                prompt,
                max_output_tokens: Some(8),
            })
            .await;
        match result {
            Ok(r) => {
                if let Ok(mut ledger) = self.ledger.lock() {
                    ledger.record(DEDUP_TIEBREAKER_GRAIN_LABEL, r.cost_cents);
                }
                r.text.trim().to_lowercase().starts_with("yes")
            }
            Err(err) => {
                tracing::warn!(error = %err, "dedup tiebreaker call failed; not merging");
                false
            }
        }
    }
}

/// Union-find with path compression; survivor preference falls out
/// of always rooting at the smallest index's smallest id downstream.
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, i: usize) -> usize {
        if self.parent[i] != i {
            let root = self.find(self.parent[i]);
            self.parent[i] = root;
        }
        self.parent[i]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // Root at the lower index — stable given a sorted input.
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent[hi] = lo;
        }
    }
}

/// Adjusted pair score: Jaro-Winkler on lowercased names + the
/// community boost/penalty. Exposed for tests.
pub fn pair_score(a: &EntityRecord, b: &EntityRecord, cfg: &DedupConfig) -> f64 {
    let jw = strsim::jaro_winkler(&a.name.to_lowercase(), &b.name.to_lowercase());
    match (a.community_id, b.community_id) {
        (Some(ca), Some(cb)) if ca == cb => (jw + cfg.community_boost).min(1.0),
        (Some(ca), Some(cb)) if ca != cb => (jw - cfg.community_penalty).max(0.0),
        _ => jw,
    }
}

/// Run the full dedup pass and return the merge plan. Deterministic:
/// records are processed sorted by id, so identical inputs always
/// produce identical groups (and thus identical survivors).
pub async fn dedup_entities(
    records: &[EntityRecord],
    cfg: &DedupConfig,
    tiebreaker: &dyn TieBreaker,
) -> Vec<MergeGroup> {
    // Sort once for determinism; work on indices into `sorted`.
    let mut sorted: Vec<&EntityRecord> = records.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));

    // Entropy gate + per-label bucketing.
    let mut buckets: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, rec) in sorted.iter().enumerate() {
        if shannon_entropy_bits(&rec.name) < cfg.entropy_min_bits {
            continue;
        }
        buckets.entry(rec.label.as_str()).or_default().push(i);
    }

    let mut uf = UnionFind::new(sorted.len());
    for indices in buckets.values() {
        // Blocking vectors once per bucket member.
        let vectors: Vec<(usize, Vec<f32>)> = indices
            .iter()
            .map(|&i| (i, ngram_vector(&sorted[i].name, cfg.ngram_dim)))
            .collect();
        for (vi, (i, vec_i)) in vectors.iter().enumerate() {
            for (j, vec_j) in vectors.iter().skip(vi + 1) {
                if cosine(vec_i, vec_j) < cfg.block_threshold {
                    continue;
                }
                let (a, b) = (sorted[*i], sorted[*j]);
                let score = pair_score(a, b, cfg);
                let matched = score >= cfg.verify_threshold
                    || (cfg.llm_tiebreaker_enabled
                        && score >= cfg.verify_threshold - cfg.ambiguous_band
                        && tiebreaker.same_entity(a, b).await);
                if matched {
                    uf.union(*i, *j);
                }
            }
        }
    }

    // Collect groups; survivor = smallest id (== first in sorted
    // order within the group, because `sorted` is id-ascending).
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..sorted.len() {
        let root = uf.find(i);
        groups.entry(root).or_default().push(i);
    }
    let mut out: Vec<MergeGroup> = groups
        .into_values()
        .filter(|members| members.len() > 1)
        .map(|mut members| {
            members.sort_unstable();
            let survivor = sorted[members[0]].id.clone();
            let merged = members[1..].iter().map(|&i| sorted[i].id.clone()).collect();
            MergeGroup { survivor, merged }
        })
        .collect();
    out.sort_by(|a, b| a.survivor.cmp(&b.survivor));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn rec(id: &str, label: &str, name: &str, community: Option<u32>) -> EntityRecord {
        EntityRecord {
            id: id.into(),
            label: label.into(),
            name: name.into(),
            community_id: community,
        }
    }

    #[tokio::test]
    async fn near_identical_names_in_same_community_merge() {
        let records = vec![
            rec("n1", "Symbol", "nexus_client", Some(1)),
            rec("n2", "Symbol", "NexusClient", Some(1)),
            rec("n3", "Symbol", "meilisearch_router", Some(2)),
        ];
        let groups = dedup_entities(&records, &DedupConfig::default(), &DisabledTieBreaker).await;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].survivor, "n1", "smallest id survives");
        assert_eq!(groups[0].merged, vec!["n2".to_string()]);
    }

    #[tokio::test]
    async fn homonyms_in_different_communities_stay_apart() {
        // The graphify separator: identical names, different
        // subsystems — the community penalty must keep them apart.
        let records = vec![
            rec("n1", "Symbol", "user_service", Some(1)),
            rec("n2", "Symbol", "user_service", Some(2)),
        ];
        let cfg = DedupConfig::default();
        let groups = dedup_entities(&records, &cfg, &DisabledTieBreaker).await;
        assert!(
            groups.is_empty(),
            "identical names in different communities must NOT merge"
        );
        // Control: same records with matching communities DO merge.
        let same = vec![
            rec("n1", "Symbol", "user_service", Some(1)),
            rec("n2", "Symbol", "user_service", Some(1)),
        ];
        let merged = dedup_entities(&same, &cfg, &DisabledTieBreaker).await;
        assert_eq!(merged.len(), 1);
    }

    #[tokio::test]
    async fn labels_never_cross_merge() {
        let records = vec![
            rec("n1", "Symbol", "graph_worker", Some(1)),
            rec("n2", "Artifact", "graph_worker", Some(1)),
        ];
        let groups = dedup_entities(&records, &DedupConfig::default(), &DisabledTieBreaker).await;
        assert!(groups.is_empty(), "a Symbol never merges with an Artifact");
    }

    #[tokio::test]
    async fn entropy_gate_skips_degenerate_names() {
        let records = vec![
            rec("n1", "Symbol", "aaaa", Some(1)),
            rec("n2", "Symbol", "aaaa", Some(1)),
        ];
        let groups = dedup_entities(&records, &DedupConfig::default(), &DisabledTieBreaker).await;
        assert!(
            groups.is_empty(),
            "zero-entropy names carry no merge signal"
        );
    }

    #[tokio::test]
    async fn union_find_is_transitive_with_stable_survivor() {
        // a≈b, b≈c → one group of three rooted at the smallest id.
        let records = vec![
            rec("n3", "Symbol", "community_detection", None),
            rec("n1", "Symbol", "community_detection", None),
            rec("n2", "Symbol", "community_detectio", None),
        ];
        let groups = dedup_entities(&records, &DedupConfig::default(), &DisabledTieBreaker).await;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].survivor, "n1");
        assert_eq!(groups[0].merged, vec!["n2".to_string(), "n3".to_string()]);
    }

    struct CountingTieBreaker {
        calls: AtomicUsize,
        answer: bool,
    }
    #[async_trait]
    impl TieBreaker for CountingTieBreaker {
        async fn same_entity(&self, _a: &EntityRecord, _b: &EntityRecord) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer
        }
    }

    #[tokio::test]
    async fn tiebreaker_consulted_only_in_ambiguous_band_and_only_when_enabled() {
        // An exact homonym across communities is the canonical
        // ambiguous pair: JW = 1.0, community penalty pulls it to
        // 0.85 — inside [0.83, 0.90). Exactly the case a human (or
        // LLM) would need to arbitrate.
        let records = vec![
            rec("n1", "Symbol", "user_service", Some(1)),
            rec("n2", "Symbol", "user_service", Some(2)),
        ];
        let base = DedupConfig::default();
        let score = pair_score(&records[0], &records[1], &base);
        assert!(
            score < base.verify_threshold && score >= base.verify_threshold - base.ambiguous_band,
            "fixture must land in the ambiguous band; got {score}"
        );

        // Flag OFF: tiebreaker never called, no merge.
        let off_tb = CountingTieBreaker {
            calls: AtomicUsize::new(0),
            answer: true,
        };
        let groups = dedup_entities(&records, &base, &off_tb).await;
        assert!(groups.is_empty());
        assert_eq!(off_tb.calls.load(Ordering::SeqCst), 0, "flag off = no call");

        // Flag ON + tiebreaker says yes: merged, exactly one call.
        let on = DedupConfig {
            llm_tiebreaker_enabled: true,
            ..base
        };
        let on_tb = CountingTieBreaker {
            calls: AtomicUsize::new(0),
            answer: true,
        };
        let groups = dedup_entities(&records, &on, &on_tb).await;
        assert_eq!(groups.len(), 1);
        assert_eq!(on_tb.calls.load(Ordering::SeqCst), 1);
    }

    struct CannedSummariser {
        text: String,
        cost: u32,
    }
    #[async_trait]
    impl crate::consolidator::summariser::Summariser for CannedSummariser {
        fn kind(&self) -> crate::consolidator::summariser::SummariserKind {
            crate::consolidator::summariser::SummariserKind::Haiku45
        }
        async fn summarise(
            &self,
            _req: crate::consolidator::summariser::SummariserRequest,
        ) -> Result<
            crate::consolidator::summariser::SummariserResult,
            crate::consolidator::summariser::SummariserError,
        > {
            Ok(crate::consolidator::summariser::SummariserResult {
                text: self.text.clone(),
                cost_cents: self.cost,
                kind: crate::consolidator::summariser::SummariserKind::Haiku45,
                input_tokens: 40,
                output_tokens: 1,
            })
        }
    }

    #[tokio::test]
    async fn summariser_tiebreaker_yes_answer_merges_and_records_cost() {
        use crate::consolidator::cost_telemetry::{CostBudget, CostLedger};
        let ledger = std::sync::Arc::new(std::sync::Mutex::new(CostLedger::default()));
        let tb = SummariserTieBreaker::new(
            std::sync::Arc::new(CannedSummariser {
                text: "yes".into(),
                cost: 3,
            }),
            ledger.clone(),
            CostBudget::default(),
        );
        let a = rec("n1", "Symbol", "user_service", Some(1));
        let b = rec("n2", "Symbol", "user_service", Some(2));
        assert!(tb.same_entity(&a, &b).await);
        let l = ledger.lock().unwrap();
        let bucket = l
            .per_grain
            .get(DEDUP_TIEBREAKER_GRAIN_LABEL)
            .expect("tiebreaker spend recorded");
        assert_eq!(bucket.cost_cents, 3);
    }

    #[tokio::test]
    async fn summariser_tiebreaker_refuses_when_budget_exhausted() {
        use crate::consolidator::cost_telemetry::{CostBudget, CostLedger};
        let ledger = std::sync::Arc::new(std::sync::Mutex::new(CostLedger::default()));
        // Zero-cent budget: can_afford(est=5) is false before any call.
        let tb = SummariserTieBreaker::new(
            std::sync::Arc::new(CannedSummariser {
                text: "yes".into(),
                cost: 3,
            }),
            ledger.clone(),
            CostBudget {
                monthly_cents_cap: 0,
            },
        );
        let a = rec("n1", "Symbol", "user_service", Some(1));
        let b = rec("n2", "Symbol", "user_service", Some(2));
        assert!(
            !tb.same_entity(&a, &b).await,
            "budget ceiling must refuse the merge, never call the model"
        );
        assert!(
            ledger.lock().unwrap().per_grain.is_empty(),
            "no spend recorded when the gate refuses"
        );
    }

    #[tokio::test]
    async fn empty_and_singleton_inputs_yield_no_groups() {
        let cfg = DedupConfig::default();
        assert!(dedup_entities(&[], &cfg, &DisabledTieBreaker)
            .await
            .is_empty());
        let one = vec![rec("n1", "Symbol", "solo_entity", None)];
        assert!(dedup_entities(&one, &cfg, &DisabledTieBreaker)
            .await
            .is_empty());
    }
}
