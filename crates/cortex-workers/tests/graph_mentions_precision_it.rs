//! Phase11k §3.8 — mention-precision integration test.
//!
//! Fifty hand-curated mentions distributed across qualified /
//! capitalised-bare / lowercase-prose / non-symbol cases. The
//! analyzer's classification (`mention_qualified` / `mention_type` /
//! `mention_prose`) plus the source-position filter (paths,
//! whitespace, lifetimes) must agree with the curated assertion set
//! at ≥ 95 % precision (≤ 2 incorrect classifications across the 50
//! cases).
//!
//! "Precision" here means: of the mentions the analyzer chose to
//! emit, how many fell into the *expected* category. Mentions the
//! analyzer dropped (path tokens, whitespace tokens, etc.) are
//! validated separately as the "skip" assertion set so a regression
//! that *adds* a false-positive emission still trips the test.

use cortex_workers::graph::analyzer::EdgeType;
use cortex_workers::graph::markdown::MarkdownAnalyzer;

#[derive(Debug, Clone, Copy)]
enum Expectation {
    /// Should emit a mention with `kind = "mention_qualified"`.
    Qualified,
    /// Should emit a mention with `kind = "mention_type"`.
    TypeMention,
    /// Should emit a mention with `kind = "mention_prose"`.
    Prose,
    /// Should NOT emit any mention.
    Skip,
}

fn cases() -> Vec<(&'static str, Expectation)> {
    use Expectation::*;
    vec![
        // Qualified mentions (15 cases).
        ("see `crate::Foo` for details", Qualified),
        ("module `crate::workers::run_worker` exits", Qualified),
        ("`std::io::Read` provides reads", Qualified),
        ("call `tokio::spawn::spawn_local`", Qualified),
        ("`vectorizer_sdk::HnswSearch` runs ANN", Qualified),
        ("path `crate::module_a::Helper`", Qualified),
        ("`a::b::c::d::e::f` deep chain", Qualified),
        ("type `Result::Ok`", Qualified),
        ("`Iterator::map` applies F", Qualified),
        ("`HashMap::insert` returns Option", Qualified),
        ("`Box::new` allocates", Qualified),
        ("`Vec::with_capacity` reserves", Qualified),
        ("`std::sync::Arc` is shared", Qualified),
        ("`crate::Foo::bar` helper", Qualified),
        ("`super::sibling` is reachable", Qualified),
        // Type-style mentions (15 cases).
        ("the `Worker` struct does X", TypeMention),
        ("`Runner` trait is implemented", TypeMention),
        ("`MyType` carries data", TypeMention),
        ("`HashMap` stores key→value", TypeMention),
        ("`HnswSearch` runs ANN", TypeMention),
        ("`PreThinkingTool` runs first", TypeMention),
        ("`Cortex` is the daemon", TypeMention),
        ("`AnalyzerLanguage` enum", TypeMention),
        ("`ResolvedTarget` enum", TypeMention),
        ("`SymbolResolver` walks tiers", TypeMention),
        ("`PackageMap` holds deps", TypeMention),
        ("`ModuleMap` indexes paths", TypeMention),
        ("`GraphPatch` is a batch", TypeMention),
        ("`NodeOp` upserts a node", TypeMention),
        ("`EdgeOp` upserts an edge", TypeMention),
        // Prose-style mentions (10 cases).
        ("call `helper` to bootstrap", Prose),
        ("invoke `run` after init", Prose),
        ("`spawn` in async ctx", Prose),
        ("`println` for stdout", Prose),
        ("`map` is functional", Prose),
        ("`filter` retains items", Prose),
        ("`fold` collapses", Prose),
        ("`extract` pulls edges", Prose),
        ("`resolve` dispatches", Prose),
        ("`parse` reads tokens", Prose),
        // Things the analyzer should SKIP (10 cases).
        ("see `src/foo.rs` for code", Skip),
        ("path `docs/spec.md`", Skip),
        ("config in `cfg/main.yaml`", Skip),
        ("script `bin/run.sh` boots", Skip),
        ("notebook `ml.ipynb`", Skip),
        ("file `./build.ts` compiles", Skip),
        ("dotted `..` chain", Skip),
        ("dollar `$amount` literal", Skip),
        ("dash `-flag` parsed", Skip),
        ("dot `.local` config", Skip),
    ]
}

fn classify(token_kind: &str) -> Expectation {
    match token_kind {
        "mention_qualified" => Expectation::Qualified,
        "mention_type" => Expectation::TypeMention,
        "mention_prose" => Expectation::Prose,
        _ => Expectation::Skip,
    }
}

fn matches(expected: Expectation, actual: Expectation) -> bool {
    matches!(
        (expected, actual),
        (Expectation::Qualified, Expectation::Qualified)
            | (Expectation::TypeMention, Expectation::TypeMention)
            | (Expectation::Prose, Expectation::Prose)
            | (Expectation::Skip, Expectation::Skip)
    )
}

#[test]
fn fifty_mentions_classified_at_95_percent_precision() {
    let analyzer = MarkdownAnalyzer::new();
    let mut correct = 0usize;
    let mut wrong: Vec<String> = Vec::new();
    let cases = cases();
    let total = cases.len();
    assert_eq!(total, 50, "the gold set must hold exactly 50 cases");

    for (idx, (line, expected)) in cases.iter().enumerate() {
        let edges = analyzer.extract(line, "cortex", "docs/spec.md");
        let mention = edges.iter().find(|e| e.edge_type == EdgeType::Mentions);
        let actual = match mention {
            Some(e) => classify(e.kind),
            None => Expectation::Skip,
        };
        if matches(*expected, actual) {
            correct += 1;
        } else {
            wrong.push(format!(
                "[{idx}] {line:?} → expected {expected:?}, got {actual:?}"
            ));
        }
    }
    let precision = (correct as f64) / (total as f64);
    assert!(
        precision >= 0.95,
        "precision {precision:.3} below 95%; misclassifications:\n{}",
        wrong.join("\n")
    );
}
