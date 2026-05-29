//! Phase18 §3.7 — branch ancestry / meili filter integration test.
//!
//! The branch ancestry walker is pure (it takes the parent map as
//! input), so the "integration" surface this IT pins is the
//! contract every retrieval lane (Meili / Vectorizer / Nexus)
//! reads:
//!
//! 1. `branch_ancestry_chain(project, branch, parent_map)` always
//!    walks leaf-first / root-last.
//! 2. `meili_branch_clause(chain)` renders the IN-disjunction the
//!    Meili filter parser expects.
//!
//! These guarantees are what the orchestrator (§3.3) relies on
//! when it hydrates the parent map once per request and threads
//! the chain into every lane's filter clause. The IT lives in
//! `tests/` (not the workers unit-test mod) so the contract is
//! exercised across the public API boundary.

use std::collections::BTreeMap;

use cortex_workers::temporal::branch_filter::{
    branch_ancestry_chain, compose_id, meili_branch_clause, DEFAULT_BRANCH,
};

fn parents(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(c, p)| ((*c).to_string(), (*p).to_string()))
        .collect()
}

#[test]
fn fork_chain_resolves_into_meili_clause_in_retrieval_order() {
    let map = parents(&[
        ("cortex:feat/x", "cortex:main"),
        ("cortex:feat/x-v2", "cortex:feat/x"),
    ]);
    let chain = branch_ancestry_chain("cortex", "feat/x-v2", &map);
    assert_eq!(
        chain,
        vec![
            compose_id("cortex", "feat/x-v2"),
            compose_id("cortex", "feat/x"),
            compose_id("cortex", "main"),
        ]
    );
    let clause = meili_branch_clause(&chain);
    assert_eq!(
        clause,
        "branch_id IN [\"cortex:feat/x-v2\", \"cortex:feat/x\", \"cortex:main\"]"
    );
}

#[test]
fn abandoned_branch_chain_still_walks_back_to_root() {
    // The walker does not inspect lifecycle — abandoned branches
    // resolve their full ancestry. The classifier (§3.3 wedge) is
    // what drops the abandoned facts from retrieval; the walker's
    // job is to make the chain visible so the audit envelope can
    // record where the hit came from.
    let map = parents(&[("cortex:feat/abandoned", "cortex:main")]);
    let chain = branch_ancestry_chain("cortex", "feat/abandoned", &map);
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0], "cortex:feat/abandoned");
    assert_eq!(chain[1], "cortex:main");
}

#[test]
fn main_only_chain_yields_singleton_clause() {
    let chain = branch_ancestry_chain("cortex", DEFAULT_BRANCH, &BTreeMap::new());
    assert_eq!(chain, vec![compose_id("cortex", DEFAULT_BRANCH)]);
    assert_eq!(meili_branch_clause(&chain), "branch_id IN [\"cortex:main\"]");
}

#[test]
fn merge_then_fork_chain_remains_deterministic() {
    // Synthetic merge/fork shape: `feat/x` was merged into `main`,
    // a new branch `feat/y` forked off `main` after the merge.
    // The walker treats each branch's parent_map entry as a single
    // pointer (it does not encode merge sources); the merge edge
    // is what the classifier consults for fold-in (§3 / ADR-021).
    // The chain for `feat/y` is unambiguous: `feat/y` → `main`.
    let map = parents(&[("cortex:feat/y", "cortex:main")]);
    let chain = branch_ancestry_chain("cortex", "feat/y", &map);
    assert_eq!(chain, vec!["cortex:feat/y", "cortex:main"]);
}
