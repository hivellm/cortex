//! Phase18 §3.5 + §3.6 — branch-aware retrieval filters.
//!
//! `branch_ancestry_chain(project, branch, parent_map)` walks the
//! `FORKED_FROM` chain to root (`main`). Every retrieval lane
//! (Meili filter, Vectorizer pre-filter, Nexus graph walk) calls it
//! once per request to build the `branch_id IN [...]` clause so a
//! branch retrieval unions facts along the ancestor chain.
//!
//! The walker is pure — it takes the parent map as input rather
//! than reading Nexus on the hot path. The orchestrator hydrates
//! the parent map once per request boundary from a cached
//! `branch -> parent` table (refresh interval set in §6 dashboard
//! wiring; default 60 s). The cache means a request never pays the
//! Nexus round-trip in line with the classifier.

use std::collections::{BTreeMap, BTreeSet};

/// Re-export so call sites refer to the same `"main"` literal.
pub use crate::graph::bitemporal::DEFAULT_BRANCH;

/// Build the chain of `<project>:<branch>` composite ids from the
/// requested branch back to root. Always terminates: a cycle (which
/// the migration backfill forbids) breaks at the first repeated
/// node.
///
/// `parent_map` keys are composite ids; values are the parent's
/// composite id. A branch with no entry is treated as root.
/// `branch` is the leaf branch's NAME (not the composite id) —
/// the helper composes the leaf id from `project` + `branch`.
///
/// Returns the chain in retrieval order: the requested branch
/// first, then each ancestor, ending with the root (`<project>:main`
/// for the canonical case).
pub fn branch_ancestry_chain(
    project: &str,
    branch: &str,
    parent_map: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let leaf = compose_id(project, branch);
    let mut cursor = leaf;
    loop {
        if seen.contains(&cursor) {
            // Cycle break — the backfill forbids cycles but the
            // walker tolerates one defensively so a poisoned graph
            // does not deadlock retrieval.
            break;
        }
        seen.insert(cursor.clone());
        out.push(cursor.clone());
        match parent_map.get(&cursor) {
            Some(parent) if !parent.is_empty() => {
                cursor = parent.clone();
            }
            _ => break,
        }
    }
    out
}

/// Compose a branch's global id from project + name. Public so the
/// orchestrator + the §4.2 CLI agree on the wire shape.
pub fn compose_id(project: &str, branch: &str) -> String {
    format!("{project}:{branch}")
}

/// Build the Meili `filter` clause for a branch retrieval. Reads
/// `branch_id IN [chain]` — the IN clause is folded by Meili into a
/// disjunction. Empty chain panics in debug, returns an empty
/// string in release (the orchestrator's invariant is "always pass
/// a non-empty chain"; the empty-string branch keeps the release
/// path defensive).
pub fn meili_branch_clause(chain: &[String]) -> String {
    if chain.is_empty() {
        debug_assert!(false, "branch_ancestry_chain must never return empty");
        return String::new();
    }
    let quoted: Vec<String> = chain
        .iter()
        .map(|id| format!("\"{}\"", id.replace('"', "\\\"")))
        .collect();
    format!("branch_id IN [{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parents(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(c, p)| (c.to_string(), p.to_string()))
            .collect()
    }

    #[test]
    fn root_main_branch_chain_has_one_entry() {
        let chain = branch_ancestry_chain("cortex", DEFAULT_BRANCH, &BTreeMap::new());
        assert_eq!(chain, vec!["cortex:main".to_string()]);
    }

    #[test]
    fn linear_chain_resolves_in_retrieval_order_leaf_first_root_last() {
        let map = parents(&[
            ("cortex:feat/x-v2", "cortex:feat/x-v1"),
            ("cortex:feat/x-v1", "cortex:main"),
        ]);
        let chain = branch_ancestry_chain("cortex", "feat/x-v2", &map);
        assert_eq!(
            chain,
            vec![
                "cortex:feat/x-v2".to_string(),
                "cortex:feat/x-v1".to_string(),
                "cortex:main".to_string(),
            ]
        );
    }

    #[test]
    fn cycle_is_broken_at_the_repeated_node() {
        let map = parents(&[
            ("cortex:feat/x", "cortex:feat/y"),
            ("cortex:feat/y", "cortex:feat/x"),
        ]);
        let chain = branch_ancestry_chain("cortex", "feat/x", &map);
        // The walker visits feat/x and feat/y once each then breaks.
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], "cortex:feat/x");
        assert_eq!(chain[1], "cortex:feat/y");
    }

    #[test]
    fn missing_parent_treats_branch_as_root() {
        let map = BTreeMap::new();
        let chain = branch_ancestry_chain("cortex", "feat/orphan", &map);
        assert_eq!(chain, vec!["cortex:feat/orphan".to_string()]);
    }

    #[test]
    fn compose_id_uses_colon_separator() {
        assert_eq!(compose_id("cortex", "main"), "cortex:main");
        assert_eq!(compose_id("cortex", "feat/x"), "cortex:feat/x");
    }

    #[test]
    fn meili_branch_clause_renders_in_disjunction_shape() {
        let chain = vec!["cortex:feat/x".to_string(), "cortex:main".to_string()];
        let clause = meili_branch_clause(&chain);
        assert_eq!(
            clause,
            "branch_id IN [\"cortex:feat/x\", \"cortex:main\"]"
        );
    }

    #[test]
    fn meili_branch_clause_escapes_embedded_quotes() {
        let chain = vec!["evil\"branch".to_string()];
        let clause = meili_branch_clause(&chain);
        assert!(clause.contains("evil\\\"branch"));
    }
}
