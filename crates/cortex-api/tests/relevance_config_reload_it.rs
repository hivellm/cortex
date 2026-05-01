//! Phase11i §3.6 — IT for the `relevance.toml` boot read + SIGHUP
//! reload path.
//!
//! Two cases:
//!
//! 1. **Boot read.** Write a temp `relevance.toml`, point
//!    [`RelevanceConfig::load_from_path`] at it, and assert the
//!    parsed values land on a fresh [`Orchestrator`] via the
//!    standard boot wiring (`with_fusion(base_fusion())`).
//! 2. **Hot reload.** Spawn an orchestrator with the original
//!    config, rewrite the TOML on disk, call
//!    [`Orchestrator::replace_fusion`] with the freshly parsed
//!    snapshot (the same call the daemon's SIGHUP task makes), and
//!    assert that the next [`Orchestrator::current_fusion`] read
//!    sees the new values. This proves the
//!    `Arc<RwLock<FusionConfig>>` plumbing is wired end-to-end —
//!    the SIGHUP signal itself is exercised by the daemon's tokio
//!    task and is not portable to Windows-hosted CI, but every
//!    other piece of the reload path is covered here.

use std::sync::Arc;

use cortex_api::lanes::{MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane};
use cortex_api::{Orchestrator, RelevanceConfig};

const ORIGINAL_TOML: &str = r#"
    [fusion]
    alpha = 0.7
    k = 60
    [cross_repo]
    boost = 0.0
    [session]
    same_session_boost = 2.0
    cohort_session_boost = 1.5
    [outcome]
    success = 1.2
    error = 0.5
    blocked_by_law = 0.3
    [recency]
    pre_change_context = 0.02
    similar_problems = 0.02
    free_search = 0.02
    explain = 0.02
    decision_lookup = 0.005
    law_check = 0.0
"#;

const RELOADED_TOML: &str = r#"
    [fusion]
    alpha = 0.4
    k = 90
    [cross_repo]
    boost = 0.6
    [session]
    same_session_boost = 3.5
    cohort_session_boost = 2.25
    [outcome]
    success = 1.4
    error = 0.4
    blocked_by_law = 0.2
    [recency]
    pre_change_context = 0.04
    similar_problems = 0.04
    free_search = 0.04
    explain = 0.04
    decision_lookup = 0.01
    law_check = 0.0
"#;

fn build_orchestrator() -> Orchestrator {
    let v = Arc::new(MemoryVectorLane::new());
    let k = Arc::new(MemoryKeywordLane::new());
    let g = Arc::new(MemoryGraphLane::new());
    Orchestrator::new(v, k, g)
}

#[test]
fn boot_read_lands_file_values_on_orchestrator() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("relevance.toml");
    std::fs::write(&path, ORIGINAL_TOML).expect("write toml");

    let cfg = RelevanceConfig::load_from_path(&path).expect("parse original toml");
    let orchestrator = build_orchestrator().with_fusion(cfg.base_fusion());

    let snap = orchestrator.current_fusion();
    assert!((snap.alpha - 0.7).abs() < f32::EPSILON);
    assert_eq!(snap.k, 60);
    assert!((snap.cross_repo_boost - 0.0).abs() < f32::EPSILON);
    assert!((snap.same_session_boost - 2.0).abs() < f32::EPSILON);
    assert!((snap.cohort_session_boost - 1.5).abs() < f32::EPSILON);
    // Per-request fields stay at the no-op defaults; they're
    // populated by per-request scope canonicalisation, not by the
    // global config file.
    assert!(snap.active_repo.is_none());
    assert!(snap.active_session_id.is_none());
    assert!(snap.outcomes_allow.is_empty());
    assert!(snap.outcomes_deny.is_empty());
    assert!((snap.recency_decay_lambda - 0.0).abs() < f32::EPSILON);
}

#[test]
fn replace_fusion_swaps_live_snapshot_for_concurrent_clones() {
    // Models the daemon's SIGHUP path:
    //   1. boot reads original.toml → with_fusion(base_fusion())
    //   2. clone the orchestrator into the SIGHUP task
    //   3. SIGHUP fires → re-read disk → replace_fusion(...)
    //   4. the *original* handle (held by QueryService) sees the
    //      new snapshot because both share the same Arc<RwLock>.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("relevance.toml");
    std::fs::write(&path, ORIGINAL_TOML).expect("write toml");

    let original = RelevanceConfig::load_from_path(&path).expect("parse original toml");
    let orchestrator = build_orchestrator().with_fusion(original.base_fusion());
    let reload_handle = orchestrator.clone();

    // Sanity — both handles agree on the original snapshot.
    let pre = orchestrator.current_fusion();
    assert!((pre.alpha - 0.7).abs() < f32::EPSILON);
    assert!((reload_handle.current_fusion().alpha - 0.7).abs() < f32::EPSILON);

    // Rewrite the file and run the reload (the same call the
    // daemon's SIGHUP task makes).
    std::fs::write(&path, RELOADED_TOML).expect("rewrite toml");
    let reloaded = RelevanceConfig::load_from_path(&path).expect("parse reloaded toml");
    reload_handle.replace_fusion(reloaded.base_fusion());

    // The *original* handle now sees the new values — the
    // Arc<RwLock<FusionConfig>> propagated the swap.
    let post = orchestrator.current_fusion();
    assert!((post.alpha - 0.4).abs() < f32::EPSILON);
    assert_eq!(post.k, 90);
    assert!((post.cross_repo_boost - 0.6).abs() < f32::EPSILON);
    assert!((post.same_session_boost - 3.5).abs() < f32::EPSILON);
    assert!((post.cohort_session_boost - 2.25).abs() < f32::EPSILON);
}

#[test]
fn missing_config_path_falls_back_to_defaults() {
    // The boot path treats "file not found" as "no override" so a
    // checkout-style deployment without the config file still
    // boots with the compile-time defaults rather than zeroing
    // every multiplier. The loader surfaces the I/O error and the
    // caller (`load_relevance_config` in main.rs) maps it to
    // RelevanceConfig::default. We replicate that policy here.
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("does-not-exist.toml");

    let resolved = RelevanceConfig::load_from_path(&missing)
        .ok()
        .unwrap_or_default();
    let orchestrator = build_orchestrator().with_fusion(resolved.base_fusion());
    let snap = orchestrator.current_fusion();
    let baseline = RelevanceConfig::default().base_fusion();
    assert_eq!(snap, baseline);
}

#[test]
fn malformed_toml_is_rejected_so_operators_see_the_typo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("relevance.toml");
    std::fs::write(&path, "fusion = { alpha = ").expect("write toml");
    let err = RelevanceConfig::load_from_path(&path).expect_err("malformed must surface");
    let msg = format!("{err}");
    assert!(msg.contains("parse"), "{msg}");
}

#[test]
fn intent_table_resolves_each_known_label() {
    // The reloaded snapshot must agree with the per-intent table
    // it ships, so the orchestrator can ask
    // `recency_lambda_for_intent(label)` and get the file value.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("relevance.toml");
    std::fs::write(&path, RELOADED_TOML).expect("write toml");
    let cfg = RelevanceConfig::load_from_path(&path).expect("parse");
    assert!((cfg.recency_lambda_for_intent("pre_change_context") - 0.04).abs() < f32::EPSILON);
    assert!((cfg.recency_lambda_for_intent("similar_problems") - 0.04).abs() < f32::EPSILON);
    assert!((cfg.recency_lambda_for_intent("free_search") - 0.04).abs() < f32::EPSILON);
    assert!((cfg.recency_lambda_for_intent("explain") - 0.04).abs() < f32::EPSILON);
    assert!((cfg.recency_lambda_for_intent("decision_lookup") - 0.01).abs() < f32::EPSILON);
    assert_eq!(cfg.recency_lambda_for_intent("law_check"), 0.0);
    // Unknown intents fall back to no decay (legacy safety).
    assert_eq!(cfg.recency_lambda_for_intent("invented"), 0.0);
}
