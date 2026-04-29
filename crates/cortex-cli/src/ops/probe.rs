//! Doctor probe mode (phase4i).
//!
//! Runs the **same** text query against the vector / keyword / graph
//! lanes and computes pairwise Jaccard overlaps on the top-K result
//! paths. Catches *semantic* drift between backends — when one
//! backend silently stops indexing a class of envelopes, the query
//! overlap collapses well before the partition counts diverge.
//!
//! The trait surface is per-lane: each [`QueryProbe`] returns a
//! deduplicated list of result paths. The doctor stitches the three
//! lanes together via [`run_query_probes`] and produces one
//! [`QueryReport`] per query that the report renderer prints.
//!
//! Live impls wrap the existing per-lane SDK clients (Meili HTTP,
//! `vectorizer-sdk::search_vectors`, `LiveNexusClient::execute_with_retry`).
//! Tests use [`MemoryQueryProbe`] which is seeded with synthetic
//! per-query top-K so the Jaccard math is exercised without booting
//! external services.

use std::collections::BTreeSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Per-lane top-K probe surface.
///
/// `search` returns the top-K result paths for `query`. Lanes that
/// can't honestly answer (empty corpus, transport failure) return
/// an empty `Vec` rather than propagating an error so a single bad
/// lane doesn't poison the whole probe run.
#[async_trait]
pub trait QueryProbe: Send + Sync {
    /// Return up to `k` deduplicated result paths for `query`.
    async fn search(&self, query: &str, k: usize) -> Vec<String>;
}

/// One pairwise Jaccard observation: `|A ∩ B| / |A ∪ B|`.
///
/// `a_size` / `b_size` carry the cardinalities so the operator can
/// distinguish "low Jaccard because both lanes returned 1 row" from
/// "low Jaccard because the lanes disagree on a 10-row top-K".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct JaccardObservation {
    /// Cardinality of lane A's result set.
    pub a_size: usize,
    /// Cardinality of lane B's result set.
    pub b_size: usize,
    /// Cardinality of the intersection.
    pub intersection: usize,
    /// Cardinality of the union.
    pub union: usize,
    /// `|A ∩ B| / |A ∪ B|`. `1.0` when both sides are empty (per the
    /// extended-set convention so an empty-corpus probe doesn't fail
    /// the run).
    pub jaccard: f64,
}

impl JaccardObservation {
    /// Compute the observation from two iterables.
    pub fn compute<A, B, S>(a: A, b: B) -> Self
    where
        A: IntoIterator<Item = S>,
        B: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let a_set: BTreeSet<String> = a.into_iter().map(|s| s.as_ref().to_string()).collect();
        let b_set: BTreeSet<String> = b.into_iter().map(|s| s.as_ref().to_string()).collect();
        let intersection = a_set.intersection(&b_set).count();
        let union_set: BTreeSet<&String> = a_set.union(&b_set).collect();
        let union = union_set.len();
        let jaccard = if union == 0 {
            1.0
        } else {
            intersection as f64 / union as f64
        };
        JaccardObservation {
            a_size: a_set.len(),
            b_size: b_set.len(),
            intersection,
            union,
            jaccard,
        }
    }
}

/// Per-query overlap report — three pairwise Jaccards plus the
/// triple intersection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryReport {
    /// The query text the operator submitted.
    pub query: String,
    /// `k` (top-K limit) the probes were asked for.
    pub k: usize,
    /// Top-K paths from the Meili lane.
    pub meili: Vec<String>,
    /// Top-K paths from the Vectorizer lane.
    pub vectorizer: Vec<String>,
    /// Top-K paths from the Nexus lane.
    pub nexus: Vec<String>,
    /// Pairwise (Vectorizer, Meili) Jaccard.
    pub vec_meili: JaccardObservation,
    /// Pairwise (Vectorizer, Nexus) Jaccard.
    pub vec_nexus: JaccardObservation,
    /// Pairwise (Meili, Nexus) Jaccard.
    pub meili_nexus: JaccardObservation,
    /// Cardinality of the three-way intersection.
    pub triple_intersection: usize,
    /// `true` when any pairwise Jaccard fell below the configured
    /// `min_overlap_jaccard` threshold AND every involved lane
    /// returned at least one result. (We don't fail when one lane
    /// is empty — that's the partition-coverage doctor's job.)
    pub below_threshold: bool,
    /// Free-text reason populated when `below_threshold` is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// In-memory `QueryProbe` for tests. Seeded with `(query, paths)`
/// pairs; unknown queries return an empty `Vec`.
#[derive(Debug, Default, Clone)]
pub struct MemoryQueryProbe {
    fixtures: std::collections::BTreeMap<String, Vec<String>>,
}

impl MemoryQueryProbe {
    /// Build an empty probe.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the response for `query`. Subsequent `search(query, k)`
    /// returns the seeded list truncated to `k`.
    pub fn seed(&mut self, query: &str, paths: Vec<String>) {
        self.fixtures.insert(query.to_string(), paths);
    }
}

#[async_trait]
impl QueryProbe for MemoryQueryProbe {
    async fn search(&self, query: &str, k: usize) -> Vec<String> {
        let mut out = self
            .fixtures
            .get(query)
            .cloned()
            .unwrap_or_default();
        out.truncate(k);
        out
    }
}

/// Run all three probes against every `query` and produce one
/// [`QueryReport`] per query. `min_overlap_jaccard` flips the
/// `below_threshold` flag when at least one pair falls below it
/// (with the both-non-empty caveat above).
pub async fn run_query_probes(
    queries: &[String],
    k: usize,
    meili: &dyn QueryProbe,
    vectorizer: &dyn QueryProbe,
    nexus: &dyn QueryProbe,
    min_overlap_jaccard: f64,
) -> Vec<QueryReport> {
    let mut out: Vec<QueryReport> = Vec::with_capacity(queries.len());
    for q in queries {
        let m = meili.search(q, k).await;
        let v = vectorizer.search(q, k).await;
        let n = nexus.search(q, k).await;
        let vec_meili = JaccardObservation::compute(v.iter().cloned(), m.iter().cloned());
        let vec_nexus = JaccardObservation::compute(v.iter().cloned(), n.iter().cloned());
        let meili_nexus = JaccardObservation::compute(m.iter().cloned(), n.iter().cloned());

        let triple_intersection = {
            let m_set: BTreeSet<&String> = m.iter().collect();
            let v_set: BTreeSet<&String> = v.iter().collect();
            let n_set: BTreeSet<&String> = n.iter().collect();
            m_set
                .iter()
                .filter(|p| v_set.contains(*p) && n_set.contains(*p))
                .count()
        };

        // A pair is suspicious only when both sides actually returned
        // something — an empty Meili (or Vectorizer or Nexus) is
        // already covered by the partition-coverage doctor and
        // would otherwise force a failed probe report on every empty
        // corpus.
        let below_threshold = pair_below(&vec_meili, min_overlap_jaccard)
            || pair_below(&vec_nexus, min_overlap_jaccard)
            || pair_below(&meili_nexus, min_overlap_jaccard);

        let reason = if below_threshold {
            let mut parts: Vec<String> = Vec::new();
            for (label, obs) in [
                ("vec/meili", &vec_meili),
                ("vec/nexus", &vec_nexus),
                ("meili/nexus", &meili_nexus),
            ] {
                if pair_below(obs, min_overlap_jaccard) {
                    parts.push(format!("{label}={:.2}", obs.jaccard));
                }
            }
            Some(format!(
                "pair(s) below threshold {min_overlap_jaccard}: {}",
                parts.join(", "),
            ))
        } else {
            None
        };

        out.push(QueryReport {
            query: q.clone(),
            k,
            meili: m,
            vectorizer: v,
            nexus: n,
            vec_meili,
            vec_nexus,
            meili_nexus,
            triple_intersection,
            below_threshold,
            reason,
        });
    }
    out
}

fn pair_below(obs: &JaccardObservation, threshold: f64) -> bool {
    obs.a_size > 0 && obs.b_size > 0 && obs.jaccard < threshold
}

/// Render a Markdown block for a single [`QueryReport`].
pub fn render_query_markdown(report: &QueryReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\n### Query: {}\n\n",
        report.query.replace('\n', " ")
    ));
    out.push_str(&format!(
        "k={}, triple_intersection={}\n\n",
        report.k, report.triple_intersection,
    ));
    out.push_str(
        "| pair | jaccard | a_size | b_size | intersection |\n\
         |------|--------:|-------:|-------:|-------------:|\n",
    );
    for (label, obs) in [
        ("vec/meili", &report.vec_meili),
        ("vec/nexus", &report.vec_nexus),
        ("meili/nexus", &report.meili_nexus),
    ] {
        out.push_str(&format!(
            "| {label} | {:.3} | {} | {} | {} |\n",
            obs.jaccard, obs.a_size, obs.b_size, obs.intersection,
        ));
    }
    if let Some(reason) = &report.reason {
        out.push_str(&format!("\n**FLAG:** {reason}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaccard_basic_round_trip() {
        let obs = JaccardObservation::compute(
            ["a", "b", "c"].iter().map(|s| s.to_string()),
            ["b", "c", "d"].iter().map(|s| s.to_string()),
        );
        assert_eq!(obs.a_size, 3);
        assert_eq!(obs.b_size, 3);
        assert_eq!(obs.intersection, 2);
        assert_eq!(obs.union, 4);
        assert!((obs.jaccard - 0.5).abs() < 1e-9);
    }

    #[test]
    fn jaccard_empty_pair_is_one_by_convention() {
        let obs: JaccardObservation = JaccardObservation::compute(
            std::iter::empty::<String>(),
            std::iter::empty::<String>(),
        );
        assert_eq!(obs.union, 0);
        assert!((obs.jaccard - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jaccard_disjoint_is_zero() {
        let obs = JaccardObservation::compute(
            ["a"].iter().map(|s| s.to_string()),
            ["b"].iter().map(|s| s.to_string()),
        );
        assert!((obs.jaccard - 0.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn memory_probe_truncates_to_k() {
        let mut probe = MemoryQueryProbe::new();
        probe.seed(
            "auth",
            ["a/1", "a/2", "a/3", "a/4"].iter().map(|s| s.to_string()).collect(),
        );
        let out = probe.search("auth", 2).await;
        assert_eq!(out, vec!["a/1".to_string(), "a/2".to_string()]);
    }

    #[tokio::test]
    async fn run_query_probes_flags_low_overlap() {
        // Vec returns A/B/C, Meili returns A/B/C — 1.0 overlap.
        // Nexus returns X/Y/Z — 0.0 overlap with vec and meili.
        let mut vec_p = MemoryQueryProbe::new();
        let mut meili_p = MemoryQueryProbe::new();
        let mut nexus_p = MemoryQueryProbe::new();
        let q = "phase4i";
        vec_p.seed(q, vec!["A".into(), "B".into(), "C".into()]);
        meili_p.seed(q, vec!["A".into(), "B".into(), "C".into()]);
        nexus_p.seed(q, vec!["X".into(), "Y".into(), "Z".into()]);

        let reports = run_query_probes(
            &[q.to_string()],
            5,
            &meili_p,
            &vec_p,
            &nexus_p,
            0.2,
        )
        .await;
        assert_eq!(reports.len(), 1);
        let r = &reports[0];
        assert!((r.vec_meili.jaccard - 1.0).abs() < 1e-9);
        assert!((r.vec_nexus.jaccard - 0.0).abs() < 1e-9);
        assert!((r.meili_nexus.jaccard - 0.0).abs() < 1e-9);
        assert_eq!(r.triple_intersection, 0);
        assert!(r.below_threshold);
        let reason = r.reason.as_deref().unwrap();
        assert!(reason.contains("vec/nexus"));
        assert!(reason.contains("meili/nexus"));
    }

    #[tokio::test]
    async fn run_query_probes_does_not_flag_when_one_lane_is_empty() {
        // Nexus is empty — pairwise Jaccards involving Nexus drop to
        // 0.0 but the empty-corpus rule keeps them out of the
        // below_threshold bucket because the partition-coverage
        // doctor already owns "lane is empty" reporting.
        let mut vec_p = MemoryQueryProbe::new();
        let mut meili_p = MemoryQueryProbe::new();
        let nexus_p = MemoryQueryProbe::new();
        let q = "auth";
        vec_p.seed(q, vec!["A".into(), "B".into()]);
        meili_p.seed(q, vec!["A".into(), "B".into()]);

        let reports = run_query_probes(
            &[q.to_string()],
            5,
            &meili_p,
            &vec_p,
            &nexus_p,
            0.2,
        )
        .await;
        assert!(!reports[0].below_threshold);
    }

    #[test]
    fn render_query_markdown_emits_pair_table() {
        let mut vec_p = MemoryQueryProbe::new();
        let mut meili_p = MemoryQueryProbe::new();
        let mut nexus_p = MemoryQueryProbe::new();
        let q = "auth";
        vec_p.seed(q, vec!["A".into(), "B".into()]);
        meili_p.seed(q, vec!["A".into(), "C".into()]);
        nexus_p.seed(q, vec!["A".into(), "D".into()]);

        let reports = futures_lite_block_on(run_query_probes(
            &[q.to_string()],
            5,
            &meili_p,
            &vec_p,
            &nexus_p,
            0.2,
        ));
        let md = render_query_markdown(&reports[0]);
        assert!(md.contains("### Query: auth"));
        assert!(md.contains("| pair | jaccard | a_size | b_size | intersection |"));
        assert!(md.contains("| vec/meili |"));
    }

    /// Tiny inline executor so the synchronous render test doesn't
    /// need its own `#[tokio::test]` ceremony.
    fn futures_lite_block_on<F: std::future::Future>(fut: F) -> F::Output {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(fut)
    }
}
