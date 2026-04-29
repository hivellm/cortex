//! Sanity-check that the canonical query-set fixture loads + meets
//! the spec coverage targets (≥10 entries per intent, all five
//! intents represented).

use std::collections::BTreeMap;
use std::path::PathBuf;

use cortex_cli::relevance_eval::queries::QuerySet;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("relevance")
        .join("queries.toml")
}

#[test]
fn canonical_fixture_parses_and_meets_coverage() {
    let path = fixture_path();
    let set = QuerySet::load(&path).expect("load canonical query set");
    assert!(
        set.queries.len() >= 50,
        "fixture must carry ≥50 queries (spec); got {}",
        set.queries.len()
    );

    let by_intent: BTreeMap<&'static str, usize> = set
        .by_intent()
        .into_iter()
        .map(|(k, v)| (k, v.len()))
        .collect();

    for required in [
        "pre_change_context",
        "decision_lookup",
        "similar_problems",
        "law_check",
        "explain",
    ] {
        let n = by_intent.get(required).copied().unwrap_or(0);
        assert!(
            n >= 10,
            "intent {required} must have ≥10 queries; got {n}"
        );
    }
}
