//! Phase27e §1.1/§1.2 — IDF-gated graph seed selection.
//!
//! `docs/analysis/graphify-comparison/08-query-ranking-idf-seeds.md`
//! documents the gap this module closes: the Nexus graph lane
//! (`crate::lanes::nexus_graph_lane`) used to bind the *whole* raw
//! query string to a single `CONTAINS $q` predicate, so a multi-word
//! natural-language query almost never matched a node's `path` /
//! `natural_key` / `title` property. This module tokenizes the query,
//! scores each token by IDF over node labels, and keeps only the
//! tokens that clear graphify's "seed only if the node scores above
//! 80% of the top score" gate — a cheap precision guard that stops a
//! common token (`error`, `handler`) from becoming a BFS seed and
//! diluting the result with a generic neighbourhood.
//!
//! Every function here is pure — the lane resolves per-token document
//! frequency via a Cypher COUNT probe and feeds the result back in as
//! a plain `Fn(&str) -> u64` closure so the scoring logic itself stays
//! fully unit-testable without a live Nexus instance.

use std::collections::HashSet;

/// Cap on the number of seed tokens fanned out to the graph lane per
/// query. Mirrors graphify's seed-gate budget: worst case is 5
/// DF-count probes (usually LRU-cached) + 5 template Cypher passes,
/// which stays comfortably inside the orchestrator's graph budget
/// share.
pub const MAX_SEEDS: usize = 5;

/// Default "keep only tokens scoring >= 80% of the top IDF" gate —
/// graphify's `_pick_seeds` threshold
/// (`docs/analysis/graphify-comparison/08-query-ranking-idf-seeds.md`).
pub const DEFAULT_TOP_GATE: f64 = 0.8;

/// Tokens shorter than this carry near-zero IDF signal and are almost
/// always noise (`"a"`, `"to"`, `"is"`).
const MIN_TOKEN_LEN: usize = 3;

/// Modest English/code stopword list. Deliberately small — this is a
/// precision guard for graph *seed* selection, not a general-purpose
/// NL stopword filter (BM25 already owns document-ranking IDF).
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "fix", "how", "what", "why", "where", "does", "did", "doing", "this",
    "that", "with", "from", "into", "use", "using", "used", "run", "get", "set", "new", "are",
    "was", "were", "can", "you", "your", "not", "but", "all",
];

/// One scored candidate seed token.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedTerm {
    /// Lowercased token text.
    pub token: String,
    /// Smoothed IDF score — higher means rarer (more specific).
    pub idf: f64,
}

/// True when `token` should never become a seed candidate: too short
/// or a stopword. Checked after lowercasing.
fn is_noise(token: &str) -> bool {
    token.len() < MIN_TOKEN_LEN || STOPWORDS.contains(&token)
}

/// Push `token` (already lowercased) onto `out` if it is not noise and
/// has not been seen yet.
fn push_unique(token: &str, seen: &mut HashSet<String>, out: &mut Vec<String>) {
    if is_noise(token) {
        return;
    }
    if seen.insert(token.to_string()) {
        out.push(token.to_string());
    }
}

/// Tokenize a free-text query into lowercase, deduplicated candidate
/// seed tokens.
///
/// Splits on any non-alphanumeric character EXCEPT `_`, so an
/// identifier like `render_edge_merge` survives whole (it may match a
/// node's `natural_key` literally). Each underscore-joined identifier
/// also has its snake_case parts (`render`, `edge`, `merge`) emitted
/// separately, deduplicated against the whole-identifier form and
/// against each other, so a query that only shares one part of a
/// compound identifier with a node still gets a candidate token.
/// Tokens shorter than 3 chars and a small stopword list are dropped.
/// An empty / all-stopword query returns an empty vector — callers
/// must handle this as "no seeds survive".
pub fn tokenize(query: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();

    for raw in query.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if raw.is_empty() {
            continue;
        }
        let lower = raw.to_ascii_lowercase();
        push_unique(&lower, &mut seen, &mut out);
        if lower.contains('_') {
            for part in lower.split('_') {
                if !part.is_empty() {
                    push_unique(part, &mut seen, &mut out);
                }
            }
        }
    }
    out
}

/// Smoothed IDF: `ln(1 + total_nodes / (1 + doc_freq))`. Rarer tokens
/// (lower `doc_freq`) score strictly higher for a fixed `total_nodes`;
/// a token matching every node (`doc_freq == total_nodes`) still
/// scores above zero (smoothing avoids a hard floor at `ln(1)`).
#[must_use]
pub fn idf(total_nodes: u64, doc_freq: u64) -> f64 {
    let total = total_nodes as f64;
    let df = doc_freq as f64;
    (1.0 + total / (1.0 + df)).ln()
}

/// Score every token by IDF, sort descending (ties broken by token
/// text ascending for determinism), then keep only the tokens whose
/// IDF is at least `top_gate * max_idf` — graphify's 80%-of-top seed
/// gate. Capped at [`MAX_SEEDS`]. Returns an empty vector when
/// `tokens` is empty (nothing to seed with).
#[must_use]
pub fn select_seeds(
    tokens: &[String],
    df_lookup: &dyn Fn(&str) -> u64,
    total_nodes: u64,
    top_gate: f64,
) -> Vec<SeedTerm> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<SeedTerm> = tokens
        .iter()
        .map(|t| SeedTerm {
            token: t.clone(),
            idf: idf(total_nodes, df_lookup(t)),
        })
        .collect();
    scored.sort_by(|a, b| {
        b.idf
            .partial_cmp(&a.idf)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.token.cmp(&b.token))
    });
    let max_idf = scored.first().map(|s| s.idf).unwrap_or(0.0);
    let gate = max_idf * top_gate;
    scored.into_iter().filter(|s| s.idf >= gate).take(MAX_SEEDS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------- tokenize ----------------

    #[test]
    fn tokenize_lowercases_and_dedups() {
        let toks = tokenize("Foo foo FOO bar");
        assert_eq!(toks, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn tokenize_drops_short_tokens_and_stopwords() {
        let toks = tokenize("a to is the and for how does this render");
        // Every token here is either < 3 chars or in the stopword
        // list, except "render".
        assert_eq!(toks, vec!["render".to_string()]);
    }

    #[test]
    fn tokenize_splits_identifiers_and_keeps_whole_form() {
        let toks = tokenize("render_edge_merge");
        assert!(toks.contains(&"render_edge_merge".to_string()));
        assert!(toks.contains(&"render".to_string()));
        assert!(toks.contains(&"edge".to_string()));
        assert!(toks.contains(&"merge".to_string()));
        // No duplicates even though parts recur across identifiers.
        let mut sorted = toks.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), toks.len());
    }

    #[test]
    fn tokenize_dedups_shared_parts_across_identifiers() {
        let toks = tokenize("render_edge_merge merge_conflict");
        let merge_count = toks.iter().filter(|t| *t == "merge").count();
        assert_eq!(merge_count, 1, "shared part must not repeat: {toks:?}");
    }

    #[test]
    fn tokenize_empty_query_is_safe() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   ").is_empty());
        assert!(tokenize("the and for").is_empty());
    }

    #[test]
    fn tokenize_splits_on_punctuation_not_underscore() {
        let toks = tokenize("FooBarService, error-handler!");
        assert!(toks.contains(&"foobarservice".to_string()));
        assert!(toks.contains(&"error".to_string()));
        assert!(toks.contains(&"handler".to_string()));
    }

    // ---------------- idf ----------------

    #[test]
    fn idf_is_monotonically_decreasing_in_doc_freq() {
        let total = 1_000;
        let rare = idf(total, 1);
        let common = idf(total, 500);
        let universal = idf(total, 1_000);
        assert!(rare > common, "rarer token must score higher: {rare} vs {common}");
        assert!(common > universal, "{common} vs {universal}");
    }

    #[test]
    fn idf_never_negative_and_handles_zero_total() {
        assert!(idf(0, 0) >= 0.0);
        assert!(idf(100, 0) >= 0.0);
    }

    // ---------------- select_seeds: 80% gate ----------------

    #[test]
    fn select_seeds_excludes_common_token_when_rare_token_dominates() {
        let tokens = vec!["error".to_string(), "foobarservice".to_string()];
        let total = 10_000u64;
        // "error" appears on 8000 of 10000 nodes (common); "foobarservice"
        // appears on exactly 1 (very rare) — the IDF gap should be large
        // enough that "error" falls under the 80% gate.
        let df = |t: &str| -> u64 {
            match t {
                "error" => 8_000,
                "foobarservice" => 1,
                _ => 0,
            }
        };
        let seeds = select_seeds(&tokens, &df, total, DEFAULT_TOP_GATE);
        let picked: Vec<&str> = seeds.iter().map(|s| s.token.as_str()).collect();
        assert_eq!(picked, vec!["foobarservice"], "common token must be gated out: {picked:?}");
    }

    #[test]
    fn select_seeds_keeps_all_when_scores_are_close() {
        let tokens = vec!["alpha".to_string(), "bravo".to_string(), "charlie".to_string()];
        let total = 1_000u64;
        // All three tokens have nearly identical (rare) document
        // frequency — none should be gated out.
        let df = |t: &str| -> u64 {
            match t {
                "alpha" => 5,
                "bravo" => 6,
                "charlie" => 5,
                _ => 0,
            }
        };
        let seeds = select_seeds(&tokens, &df, total, DEFAULT_TOP_GATE);
        assert_eq!(seeds.len(), 3, "close scores must all clear the gate: {seeds:?}");
    }

    #[test]
    fn select_seeds_caps_at_max_seeds() {
        let tokens: Vec<String> = (0..10).map(|i| format!("token{i}")).collect();
        let total = 1_000u64;
        // Every token equally rare — all would clear the gate, so the
        // cap is the only thing limiting the result.
        let df = |_: &str| -> u64 { 1 };
        let seeds = select_seeds(&tokens, &df, total, DEFAULT_TOP_GATE);
        assert_eq!(seeds.len(), MAX_SEEDS);
    }

    #[test]
    fn select_seeds_empty_tokens_is_safe() {
        let df = |_: &str| -> u64 { 0 };
        assert!(select_seeds(&[], &df, 1_000, DEFAULT_TOP_GATE).is_empty());
    }

    #[test]
    fn select_seeds_is_deterministic_across_calls() {
        let tokens = vec!["bravo".to_string(), "alpha".to_string(), "charlie".to_string()];
        let total = 1_000u64;
        let df = |t: &str| -> u64 {
            match t {
                "alpha" => 5,
                "bravo" => 5,
                "charlie" => 5,
                _ => 0,
            }
        };
        let first = select_seeds(&tokens, &df, total, DEFAULT_TOP_GATE);
        let second = select_seeds(&tokens, &df, total, DEFAULT_TOP_GATE);
        assert_eq!(first, second);
        // Equal IDF scores tie-break on token text ascending.
        let picked: Vec<&str> = first.iter().map(|s| s.token.as_str()).collect();
        assert_eq!(picked, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn select_seeds_sorts_by_idf_descending() {
        let tokens = vec!["common".to_string(), "rare".to_string()];
        let total = 10_000u64;
        let df = |t: &str| -> u64 {
            match t {
                "common" => 9_000,
                "rare" => 2,
                _ => 0,
            }
        };
        let seeds = select_seeds(&tokens, &df, total, 0.0);
        assert_eq!(seeds[0].token, "rare", "higher-IDF token must sort first");
    }
}
