//! Harness — fetches snippets per query, derives per-snippet doc ids,
//! and scores `recall@10` + `MRR` against the labeled expected ids.
//!
//! The fetch path is split out behind [`SnippetFetcher`] so unit
//! tests can drive the scoring math without a live `cortex-api`.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use cortex_api::types::{QueryRequest, QueryResponse, Snippet};
use serde::{Deserialize, Serialize};

use super::queries::{LabeledQuery, QuerySet};
use super::report::{IntentScores, QueryResult, RelevanceReport};

/// One scored query — the in-memory shape the report module turns
/// into the persisted [`QueryResult`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoredQuery {
    /// Stable fixture id.
    pub id: String,
    /// Intent label.
    pub intent: String,
    /// Query text.
    pub query: String,
    /// Was *any* expected id in the top-10 fused snippets?
    pub recall_at_10: bool,
    /// 1-based rank of the first match.
    pub matched_rank: Option<usize>,
    /// `1.0 / matched_rank` or `0.0`.
    pub mrr: f64,
    /// Which expected id matched first (helps triage).
    pub matched_doc_id: Option<String>,
    /// Number of snippets returned for the query.
    pub returned: usize,
}

impl From<ScoredQuery> for QueryResult {
    fn from(s: ScoredQuery) -> Self {
        QueryResult {
            id: s.id,
            intent: s.intent,
            query: s.query,
            recall_at_10: s.recall_at_10,
            matched_rank: s.matched_rank,
            mrr: s.mrr,
            matched_doc_id: s.matched_doc_id,
            returned: s.returned,
        }
    }
}

/// CLI-shaped options consumed by [`run_harness`].
#[derive(Debug, Clone)]
pub struct HarnessOptions {
    /// Base URL of the running `cortex-api`.
    pub api_url: String,
    /// Per-query budget (ms) — propagated as `budget_ms` and used as
    /// the HTTP request timeout.
    pub budget_ms: u64,
    /// Top-k that drives recall@k. Spec calls for `10`.
    pub top_k: usize,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        Self {
            api_url: "http://127.0.0.1:17000".to_string(),
            budget_ms: 1_500,
            top_k: 10,
        }
    }
}

/// Pluggable fetcher — production runs against the HTTP client; tests
/// drive a deterministic in-memory implementation.
#[async_trait::async_trait]
pub trait SnippetFetcher: Send + Sync {
    /// Return the snippet list for a single query.
    async fn fetch(&self, query: &LabeledQuery) -> Result<QueryResponse>;
    /// Best-effort backend snapshot used for omission. Default impl
    /// returns an "all-healthy" snapshot — HTTP fetcher overrides.
    async fn status_snapshot(&self) -> StatusSnapshot {
        StatusSnapshot::all_healthy()
    }
}

/// Snapshot used by the omission step — derived from `/v1/status`.
#[derive(Debug, Clone, Default)]
pub struct StatusSnapshot {
    /// Repos the daemon currently has signal for. Empty → unknown.
    pub indexed_repos: BTreeSet<String>,
    /// `cortex-api` crate version surfaced for the report header.
    pub api_version: Option<String>,
    /// `true` when the harness could not reach `/v1/status`.
    pub unreachable: bool,
}

impl StatusSnapshot {
    fn all_healthy() -> Self {
        Self::default()
    }
}

/// Reqwest-backed fetcher — what the CLI actually uses.
pub struct HttpFetcher {
    client: reqwest::Client,
    base: String,
    budget_ms: u64,
}

impl HttpFetcher {
    /// Build a fetcher pointing at `base` with the given per-request budget.
    pub fn new(base: impl Into<String>, budget_ms: u64) -> Result<Self> {
        let client = reqwest::Client::builder()
            // Add 1s headroom over the request budget so transport
            // jitter doesn't cause spurious timeouts.
            .timeout(std::time::Duration::from_millis(budget_ms + 1_000))
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            client,
            base: base.into(),
            budget_ms,
        })
    }
}

#[async_trait::async_trait]
impl SnippetFetcher for HttpFetcher {
    async fn fetch(&self, query: &LabeledQuery) -> Result<QueryResponse> {
        let req = QueryRequest {
            intent: query.intent,
            scope: query.scope.clone(),
            query: query.query.clone(),
            limit: 20,
            k: 50,
            include: query.include.clone().unwrap_or_else(default_include),
            budget_ms: self.budget_ms,
            budget_bytes: None,
            as_of: None,
            branch: None,
            projects: None,
            include_history: None,
            include_future: None,
            include_branches: None,
        };
        let url = format!("{}/v1/query", self.base.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .header("x-cortex-caller", "cortex-relevance-eval")
            .json(&req)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("query {} returned {status}: {body}", query.id);
        }
        let parsed: QueryResponse = resp
            .json()
            .await
            .with_context(|| format!("parse response for {}", query.id))?;
        Ok(parsed)
    }

    async fn status_snapshot(&self) -> StatusSnapshot {
        let url = format!("{}/v1/status", self.base.trim_end_matches('/'));
        match self.client.get(&url).send().await {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(v) => {
                    let indexed_repos: BTreeSet<String> = v
                        .get("indexed_repos")
                        .and_then(|x| x.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|s| s.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let api_version = v.get("version").and_then(|s| s.as_str()).map(String::from);
                    StatusSnapshot {
                        indexed_repos,
                        api_version,
                        unreachable: false,
                    }
                }
                Err(_) => StatusSnapshot::all_healthy(),
            },
            Err(err) => {
                tracing::warn!(error = %err, "status endpoint unreachable; recording omission");
                StatusSnapshot {
                    unreachable: true,
                    ..Default::default()
                }
            }
        }
    }
}

fn default_include() -> Vec<cortex_api::types::IncludeField> {
    use cortex_api::types::IncludeField;
    vec![
        IncludeField::Snippets,
        IncludeField::Decisions,
        IncludeField::Violations,
        IncludeField::GraphNeighbors,
        IncludeField::SimilarTurns,
    ]
}

/// Derive a stable doc-id string from a snippet for the recall match.
///
/// Composite shape: `{repo|"_"}|{path|"_"}|{content_hash|"_"}`. The
/// match step compares each expected id against this canonical id
/// AND each individual field — so fixture authors can use a path
/// (`crates/foo/src/lib.rs`), a content hash (`sha256:abc...`), the
/// composite, or a substring matcher.
pub fn doc_id_for(s: &Snippet) -> String {
    format!(
        "{}|{}|{}",
        s.repo.as_deref().unwrap_or("_"),
        s.path.as_deref().unwrap_or("_"),
        s.content_hash.as_deref().unwrap_or("_"),
    )
}

/// Fields a snippet exposes for matching against an expected id.
fn snippet_match_fields(s: &Snippet) -> [Option<&str>; 5] {
    [
        s.repo.as_deref(),
        s.path.as_deref(),
        s.symbol.as_deref(),
        s.content_hash.as_deref(),
        s.collection.as_deref(),
    ]
}

/// Returns the 1-based rank of the first snippet that matches any
/// expected id, or `None` when nothing matches in the top-`k`. The
/// matcher tries (in order):
///   1. exact equality with the canonical doc id;
///   2. exact equality with `repo` / `path` / `symbol` / `content_hash` / `collection`;
///   3. substring match against `path` / `symbol` (last-resort, lets
///      curators use partial paths like `strategies.rs` without
///      worrying about a chunk hash suffix).
///
/// Returns the matched expected id alongside the rank.
pub fn first_match_rank(
    snippets: &[Snippet],
    expected: &[String],
    top_k: usize,
) -> Option<(usize, String)> {
    let limit = snippets.len().min(top_k);
    for (idx, snippet) in snippets[..limit].iter().enumerate() {
        let canonical = doc_id_for(snippet);
        for exp in expected {
            if exp == &canonical {
                return Some((idx + 1, exp.clone()));
            }
            for field in snippet_match_fields(snippet).iter().flatten() {
                if exp == field {
                    return Some((idx + 1, exp.clone()));
                }
            }
            // Substring fall-back — only against path / symbol so we
            // never accidentally match arbitrary text content.
            for field in [snippet.path.as_deref(), snippet.symbol.as_deref()]
                .iter()
                .flatten()
            {
                if !exp.is_empty() && field.contains(exp.as_str()) {
                    return Some((idx + 1, exp.clone()));
                }
            }
        }
    }
    None
}

/// Score a single labeled query against a fetched response.
pub fn score_one(query: &LabeledQuery, response: &QueryResponse, top_k: usize) -> ScoredQuery {
    let snippets = &response.results.snippets;
    let returned = snippets.len();
    let matched = first_match_rank(snippets, &query.expected_doc_ids, top_k);
    let (matched_rank, matched_doc_id, mrr, recall_at_10) = match matched {
        Some((rank, id)) => (Some(rank), Some(id), 1.0 / rank as f64, true),
        None => (None, None, 0.0, false),
    };
    ScoredQuery {
        id: query.id.clone(),
        intent: query.intent_label().to_string(),
        query: query.query.clone(),
        recall_at_10,
        matched_rank,
        mrr,
        matched_doc_id,
        returned,
    }
}

/// Run the full harness — fetch every query, score, aggregate, and
/// produce the persisted report.
pub async fn run_harness<F: SnippetFetcher>(
    fetcher: &F,
    set: &QuerySet,
    opts: &HarnessOptions,
    git_sha: &str,
) -> Result<RelevanceReport> {
    let snapshot = fetcher.status_snapshot().await;
    let mut omitted_intents: BTreeSet<String> = BTreeSet::new();

    if snapshot.unreachable {
        tracing::warn!("/v1/status unreachable — recording every intent as omitted");
        for intent_label in set.by_intent().keys() {
            omitted_intents.insert((*intent_label).to_string());
        }
        let report = RelevanceReport {
            generated_at: chrono::Utc::now().to_rfc3339(),
            git_sha: git_sha.to_string(),
            api_version: None,
            omitted_intents: omitted_intents.into_iter().collect(),
            per_intent: BTreeMap::new(),
            global: IntentScores::default(),
            queries: Vec::new(),
        };
        return Ok(report);
    }

    let mut scored: Vec<ScoredQuery> = Vec::with_capacity(set.queries.len());
    let mut per_intent: BTreeMap<String, Vec<ScoredQuery>> = BTreeMap::new();

    for query in &set.queries {
        if !snapshot.indexed_repos.is_empty() {
            if let Some(scope_repo) = &query.scope.repo {
                if !snapshot.indexed_repos.contains(scope_repo) {
                    tracing::warn!(
                        query_id = %query.id,
                        scope_repo = %scope_repo,
                        "scope repo not in indexed_repos snapshot — omitting intent bucket"
                    );
                    omitted_intents.insert(query.intent_label().to_string());
                    continue;
                }
            }
        }
        match fetcher.fetch(query).await {
            Ok(resp) => {
                let row = score_one(query, &resp, opts.top_k);
                per_intent
                    .entry(row.intent.clone())
                    .or_default()
                    .push(row.clone());
                scored.push(row);
            }
            Err(err) => {
                tracing::warn!(query_id = %query.id, error = %err, "query failed; counting as miss");
                let miss = ScoredQuery {
                    id: query.id.clone(),
                    intent: query.intent_label().to_string(),
                    query: query.query.clone(),
                    recall_at_10: false,
                    matched_rank: None,
                    mrr: 0.0,
                    matched_doc_id: None,
                    returned: 0,
                };
                per_intent
                    .entry(miss.intent.clone())
                    .or_default()
                    .push(miss.clone());
                scored.push(miss);
            }
        }
    }

    let global = IntentScores::from_scored(&scored);
    let per_intent_scores: BTreeMap<String, IntentScores> = per_intent
        .iter()
        .map(|(intent, rows)| (intent.clone(), IntentScores::from_scored(rows)))
        .collect();

    // Stable id ordering — diffs across runs touch only the rows
    // that actually changed.
    let mut queries: Vec<QueryResult> = scored.into_iter().map(QueryResult::from).collect();
    queries.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(RelevanceReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        git_sha: git_sha.to_string(),
        api_version: snapshot.api_version,
        omitted_intents: omitted_intents.into_iter().collect(),
        per_intent: per_intent_scores,
        global,
        queries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_api::types::{
        BudgetReport, DebugInfo, Intent, QueryResponse, ResultsBag, Scope, Snippet,
    };

    fn snip(rank: usize, path: &str, hash: Option<&str>) -> Snippet {
        Snippet {
            rank,
            source: "vector".into(),
            collection: Some("cortex-Cortex-code".into()),
            repo: Some("Cortex".into()),
            path: Some(path.into()),
            symbol: None,
            content_hash: hash.map(String::from),
            text: format!("snippet for {path}"),
            body_truncated: false,
            score: 1.0 / rank as f64,
            why: None,
        }
    }

    fn lq(id: &str, expected: Vec<&str>) -> LabeledQuery {
        LabeledQuery {
            id: id.into(),
            intent: Intent::Explain,
            scope: Scope::default(),
            query: format!("q-{id}"),
            expected_doc_ids: expected.into_iter().map(String::from).collect(),
            notes: None,
            include: None,
        }
    }

    fn response(snippets: Vec<Snippet>) -> QueryResponse {
        QueryResponse {
            intent: "explain".into(),
            query_id: "01TEST".into(),
            scope_resolved: Scope::default(),
            results: ResultsBag {
                snippets,
                ..Default::default()
            },
            laws_active: Vec::new(),
            budget: BudgetReport {
                used_ms: 0,
                cap_ms: 500,
                cache: "miss".into(),
            },
            debug: DebugInfo::default(),
            notice: None,
            clipped: None,
        }
    }

    #[test]
    fn first_match_returns_rank_one_for_first_position() {
        let snippets = vec![
            snip(1, "crates/cortex-api/src/strategies.rs", Some("sha256:aaa")),
            snip(2, "crates/cortex-api/src/lanes.rs", Some("sha256:bbb")),
        ];
        let m = first_match_rank(
            &snippets,
            &["crates/cortex-api/src/strategies.rs".to_string()],
            10,
        );
        assert_eq!(m.as_ref().map(|(r, _)| *r), Some(1));
    }

    #[test]
    fn first_match_substring_match_on_path() {
        let snippets = vec![snip(1, "crates/cortex-api/src/strategies.rs", None)];
        let m = first_match_rank(&snippets, &["strategies.rs".to_string()], 10);
        assert_eq!(m.as_ref().map(|(r, _)| *r), Some(1));
    }

    #[test]
    fn first_match_misses_below_top_k() {
        let snippets = (1..=15)
            .map(|i| snip(i, &format!("path-{i}.rs"), None))
            .collect::<Vec<_>>();
        // The match exists at rank 12 but top_k=10 cuts it.
        let m = first_match_rank(&snippets, &["path-12.rs".to_string()], 10);
        assert_eq!(m, None);
    }

    #[test]
    fn first_match_handles_empty_snippets() {
        let m = first_match_rank(&[], &["anything".to_string()], 10);
        assert_eq!(m, None);
    }

    #[test]
    fn first_match_picks_first_when_two_match() {
        let snippets = vec![
            snip(1, "a.rs", Some("sha256:aaa")),
            snip(2, "b.rs", Some("sha256:bbb")),
        ];
        let m = first_match_rank(&snippets, &["b.rs".to_string(), "a.rs".to_string()], 10);
        // a.rs is at rank 1 — should win even though it appears
        // second in the expected list.
        assert_eq!(m.as_ref().map(|(r, _)| *r), Some(1));
        assert_eq!(m.as_ref().map(|(_, id)| id.as_str()), Some("a.rs"));
    }

    #[test]
    fn score_one_recall_and_mrr() {
        let q = lq("rel-001", vec!["a.rs"]);
        let resp = response(vec![snip(1, "x.rs", None), snip(2, "a.rs", None)]);
        let s = score_one(&q, &resp, 10);
        assert!(s.recall_at_10);
        assert_eq!(s.matched_rank, Some(2));
        assert!((s.mrr - 0.5).abs() < 1e-9);
    }

    #[test]
    fn score_one_miss_returns_zero_mrr() {
        let q = lq("rel-002", vec!["nope.rs"]);
        let resp = response(vec![snip(1, "x.rs", None)]);
        let s = score_one(&q, &resp, 10);
        assert!(!s.recall_at_10);
        assert_eq!(s.mrr, 0.0);
        assert_eq!(s.matched_rank, None);
    }

    #[test]
    fn doc_id_for_synthesizes_composite() {
        let s = snip(1, "x.rs", Some("sha256:abc"));
        assert_eq!(doc_id_for(&s), "Cortex|x.rs|sha256:abc");
    }

    // ---- Fake fetcher for end-to-end run ----

    struct FakeFetcher {
        per_query: BTreeMap<String, QueryResponse>,
    }

    #[async_trait::async_trait]
    impl SnippetFetcher for FakeFetcher {
        async fn fetch(&self, query: &LabeledQuery) -> Result<QueryResponse> {
            self.per_query
                .get(&query.id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no fake response for {}", query.id))
        }
    }

    #[tokio::test]
    async fn run_harness_aggregates_global_and_per_intent() {
        let mut per_query = BTreeMap::new();
        per_query.insert("rel-001".to_string(), response(vec![snip(1, "a.rs", None)]));
        per_query.insert("rel-002".to_string(), response(vec![snip(1, "x.rs", None)]));
        let fetcher = FakeFetcher { per_query };

        let set = QuerySet {
            version: 1,
            queries: vec![
                lq("rel-001", vec!["a.rs"]),    // hit
                lq("rel-002", vec!["nope.rs"]), // miss
            ],
        };

        let report = run_harness(&fetcher, &set, &HarnessOptions::default(), "deadbeef")
            .await
            .unwrap();

        assert_eq!(report.global.total, 2);
        assert_eq!(report.global.matches, 1);
        assert!((report.global.recall_at_10_pct - 50.0).abs() < 1e-9);
        // MRR = (1.0 + 0.0) / 2 = 0.5
        assert!((report.global.mrr_avg - 0.5).abs() < 1e-9);
        // Per-intent: both Explain — single bucket.
        assert_eq!(report.per_intent.get("explain").unwrap().total, 2);
        // Stable id ordering.
        assert_eq!(report.queries[0].id, "rel-001");
        assert_eq!(report.queries[1].id, "rel-002");
    }

    #[test]
    fn first_match_exact_content_hash_field() {
        let snippets = vec![snip(1, "x.rs", Some("sha256:abc"))];
        let m = first_match_rank(&snippets, &["sha256:abc".to_string()], 10);
        assert_eq!(m.as_ref().map(|(r, _)| *r), Some(1));
    }

    #[test]
    fn first_match_exact_collection_field() {
        let snippets = vec![snip(1, "x.rs", None)];
        let m = first_match_rank(&snippets, &["cortex-Cortex-code".to_string()], 10);
        assert_eq!(m.as_ref().map(|(r, _)| *r), Some(1));
    }

    #[test]
    fn first_match_substring_against_symbol() {
        let mut s = snip(1, "x.rs", None);
        s.symbol = Some("PreThinkingTool".into());
        let m = first_match_rank(&[s], &["ThinkingTool".to_string()], 10);
        assert_eq!(m.as_ref().map(|(r, _)| *r), Some(1));
    }

    #[test]
    fn first_match_ignores_empty_expected_string() {
        let snippets = vec![snip(1, "x.rs", None)];
        let m = first_match_rank(&snippets, &["".to_string()], 10);
        assert_eq!(m, None);
    }

    #[test]
    fn scored_query_into_query_result_round_trip() {
        let s = ScoredQuery {
            id: "rel-001".into(),
            intent: "explain".into(),
            query: "q".into(),
            recall_at_10: true,
            matched_rank: Some(3),
            mrr: 1.0 / 3.0,
            matched_doc_id: Some("path.rs".into()),
            returned: 8,
        };
        let q: QueryResult = s.clone().into();
        assert_eq!(q.id, s.id);
        assert_eq!(q.intent, s.intent);
        assert_eq!(q.matched_rank, Some(3));
        assert_eq!(q.matched_doc_id.as_deref(), Some("path.rs"));
        assert_eq!(q.returned, 8);
    }

    #[test]
    fn harness_options_default_uses_spec_constants() {
        let o = HarnessOptions::default();
        assert_eq!(o.api_url, "http://127.0.0.1:17000");
        assert_eq!(o.budget_ms, 1_500);
        assert_eq!(o.top_k, 10);
    }

    #[test]
    fn http_fetcher_new_succeeds() {
        // Constructor builds a reqwest client + remembers the base
        // URL. We only assert it constructs cleanly — actual HTTP
        // behaviour is exercised against a live cortex-api by the
        // CI relevance gate.
        let f = HttpFetcher::new("http://127.0.0.1:17000", 500).expect("HttpFetcher");
        // Smoke: the constructed type is Send + Sync (compile-time
        // gate on SnippetFetcher trait bounds).
        fn assert_sf<T: SnippetFetcher>(_: &T) {}
        assert_sf(&f);
    }

    #[tokio::test]
    async fn fake_fetcher_default_status_snapshot_is_all_healthy() {
        // The default trait impl returns an empty StatusSnapshot
        // (no indexed_repos, not unreachable). Cover by using a
        // fetcher that doesn't override `status_snapshot`.
        struct EmptyFetcher;
        #[async_trait::async_trait]
        impl SnippetFetcher for EmptyFetcher {
            async fn fetch(&self, _q: &LabeledQuery) -> Result<QueryResponse> {
                anyhow::bail!("not used")
            }
        }
        let f = EmptyFetcher;
        let snap = f.status_snapshot().await;
        assert!(snap.indexed_repos.is_empty());
        assert!(!snap.unreachable);
        assert!(snap.api_version.is_none());
    }

    #[tokio::test]
    async fn fetch_failure_records_zero_score_row() {
        // FakeFetcher returns Err for unknown ids → run_harness must
        // record a miss row with mrr=0 and continue.
        let fetcher = FakeFetcher {
            per_query: BTreeMap::new(),
        };
        let set = QuerySet {
            version: 1,
            queries: vec![lq("rel-001", vec!["a.rs"])],
        };
        let report = run_harness(&fetcher, &set, &HarnessOptions::default(), "sha")
            .await
            .unwrap();
        assert_eq!(report.global.total, 1);
        assert_eq!(report.global.matches, 0);
        let row = &report.queries[0];
        assert!(!row.recall_at_10);
        assert_eq!(row.mrr, 0.0);
        assert_eq!(row.returned, 0);
    }
}
