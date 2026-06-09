//! Integration tests for the bootstrap CLI runner — covers the
//! spec-09 acceptance criteria that don't require a live Synap /
//! Vectorizer / Nexus / Meili instance.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use cortex_cli::bootstrap::{
    estimate_repo, run_repo, Checkpoint, CortexSection, MemoryPublisher, Metrics, Publisher,
    RunnerConfig,
};
use serde_json::Value;
use tempfile::TempDir;

/// Build a tiny synthetic repo on disk with a code file, a doc, an
/// ADR, a law file, a memory note, and a `.env` carrying a synthetic
/// secret. Returns the temp dir handle so the caller controls
/// lifetime.
fn make_fixture_repo() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("docs/decisions")).unwrap();
    fs::create_dir_all(root.join("rulebook/laws")).unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub fn hnsw_search(k: usize) -> usize { k }\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "# Vectorizer\n\nVector store.\n").unwrap();
    fs::write(
        root.join("docs/decisions/0042-adopt-meili.md"),
        "# Adopt Meilisearch\n\nStatus: accepted\nSupersedes: ADR-0001\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        root.join("rulebook/laws/LAW-007.yaml"),
        "law_id: LAW-007\ntitle: No skipping hooks\nseverity: critical\ndetector: hook:pre_commit_no_skip\n",
    )
    .unwrap();
    fs::write(
        root.join("CLAUDE.md"),
        "# CLAUDE memory\n\nAlways respect the laws.\n",
    )
    .unwrap();
    // Synthetic .env carrying a fake AWS-style secret.
    fs::write(
        root.join(".env"),
        "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLEAAAAA\n",
    )
    .unwrap();
    tmp
}

/// Spec-09-shaped `cortex.toml` body for the fixture above.
fn fixture_config() -> CortexSection {
    let body = r#"
[cortex]
id = "Fixture"

[cortex.decisions]
promote_patterns = ["docs/decisions/*.md"]

[cortex.laws]
promote_patterns = ["rulebook/laws/*.yaml"]

[cortex.memories]
import_files = ["CLAUDE.md"]

[cortex.git]
include_commits = false
"#;
    let parsed: cortex_cli::bootstrap::CortexToml = toml::from_str(body).unwrap();
    parsed.cortex
}

#[tokio::test]
async fn estimate_mode_walks_without_writes() {
    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let est = estimate_repo(repo.path(), "Fixture", &cfg);
    assert!(
        est.files_kept >= 4,
        "fixture must keep code+doc+adr+law+memory"
    );
    assert!(est.events_total > 0);
    assert_eq!(est.commits, 0, "fixture has no .git");
}

#[tokio::test]
async fn end_to_end_emits_one_event_per_recognised_kind() {
    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let metrics = Arc::new(Metrics::new());
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };
    let report = run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        metrics.clone(),
        &mut checkpoint,
        None,
        None,
    )
    .await
    .expect("run_repo");

    // phase10d — `repo_id` lowercased on the canonical wire form.
    assert_eq!(report.repo_id, "fixture");
    assert!(report.events_published >= 4);

    // Spec-09 promotions must produce one event each.
    assert_eq!(publisher.by_kind("decision.imported").len(), 1);
    assert_eq!(publisher.by_kind("law.imported").len(), 1);
    assert_eq!(publisher.by_kind("memory.imported").len(), 1);
    assert!(!publisher.by_kind("artifact.code").is_empty());
    assert!(!publisher.by_kind("artifact.doc").is_empty());

    // Checkpoint marks the repo done.
    assert!(checkpoint.is_repo_done("fixture"));
}

#[tokio::test]
async fn idempotent_replay_reuses_checkpoint_resume() {
    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let metrics = Arc::new(Metrics::new());
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };
    run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn.clone(),
        metrics.clone(),
        &mut checkpoint,
        None,
        None,
    )
    .await
    .unwrap();
    let after_first = publisher.count();

    // Resume with last_file=last seen file. The runner should treat
    // every entry <= last_file as already-published and emit nothing.
    let last_file = checkpoint
        .repos
        .get("fixture")
        .and_then(|p| p.last_file.clone());
    let publisher2 = Arc::new(MemoryPublisher::new());
    let pub_dyn2: Arc<dyn Publisher> = publisher2.clone();
    let mut checkpoint2 = Checkpoint::new("now".into());
    run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn2,
        Arc::new(Metrics::new()),
        &mut checkpoint2,
        last_file,
        None,
    )
    .await
    .unwrap();

    assert!(
        publisher2.count() <= 1,
        "resume past last_file emits at most stragglers"
    );
    let _ = after_first;
}

#[tokio::test]
async fn redaction_strips_synthetic_secret_from_env() {
    let repo = make_fixture_repo();
    // Promote the .env so the file actually surfaces as a memory event
    // (otherwise the walker classifies it as Other and drops it).
    let mut cfg = fixture_config();
    cfg.memories.import_files.push(".env".to_string());
    cfg.git.include_commits = false;

    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };
    run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        Arc::new(Metrics::new()),
        &mut checkpoint,
        None,
        None,
    )
    .await
    .unwrap();

    // Locate the memory event for `.env` and assert the AWS-key
    // bytes are gone.
    let envs = publisher.by_kind("memory.imported");
    let env_doc = envs
        .iter()
        .find(|e| e.source["path"] == Value::String(".env".to_string()))
        .expect("memory.imported for .env");
    let body = env_doc.redacted_payload["body"].as_str().expect("body str");
    assert!(
        !body.contains("AKIAIOSFODNN7EXAMPLEAAAAA"),
        "synthetic AWS key must be redacted: {body}"
    );
    assert!(env_doc.redactions >= 1);
}

#[tokio::test]
async fn dry_run_publishes_nothing_to_the_publisher() {
    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: true,
        kind_filter: Vec::new(),
    };
    let report = run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        Arc::new(Metrics::new()),
        &mut checkpoint,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(report.events_published > 0, "dry-run still walks");
    assert_eq!(publisher.count(), 0, "dry-run never publishes");
}

#[tokio::test]
async fn parallel_runner_executes_all_repos_with_no_event_loss() {
    use cortex_cli::bootstrap::run_repos_parallel;

    let repos: Vec<TempDir> = (0..4).map(|_| make_fixture_repo()).collect();
    let publisher = Arc::new(MemoryPublisher::new());
    let metrics = Arc::new(Metrics::new());

    // Stage one future per repo. Each future writes into the shared
    // memory publisher.
    let mut futures = Vec::with_capacity(repos.len());
    for (idx, dir) in repos.iter().enumerate() {
        let path = dir.path().to_path_buf();
        let cfg = fixture_config();
        let pub_arc: Arc<dyn Publisher> = publisher.clone();
        let metrics = metrics.clone();
        let runner_cfg = RunnerConfig {
            repo_id: format!("Fixture-{idx}"),
            stream: "cortex.events.bootstrap".into(),
            since: None,
            dry_run: false,
            kind_filter: Vec::new(),
        };
        futures.push(async move {
            let mut checkpoint = Checkpoint::new("now".into());
            run_repo(
                &path,
                &runner_cfg,
                &cfg,
                pub_arc,
                metrics,
                &mut checkpoint,
                None,
                None,
            )
            .await
        });
    }
    let outcomes = run_repos_parallel(2, futures).await;
    let mut total_events = 0u64;
    for o in outcomes {
        let report = o.expect("repo run");
        total_events += report.events_published;
    }
    // 4 repos × ≥4 events each
    assert!(total_events >= 16, "got {total_events}");
    assert_eq!(
        publisher.count() as u64,
        total_events,
        "publisher count must match the sum of per-repo reports"
    );
}

#[tokio::test]
async fn checkpoint_persists_atomically_to_disk() {
    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };
    run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        Arc::new(Metrics::new()),
        &mut checkpoint,
        None,
        None,
    )
    .await
    .unwrap();
    let cp_path = repo.path().join(".cortex-bootstrap.state.json");
    cortex_cli::bootstrap::write_atomic(&cp_path, &checkpoint).unwrap();
    let reloaded = cortex_cli::bootstrap::load_checkpoint(&cp_path).unwrap();
    assert!(reloaded.is_repo_done("fixture"));
    let progress = reloaded.repos.get("fixture").unwrap();
    assert!(progress.events_emitted >= 4);
    assert!(progress.last_file.is_some());
}

#[tokio::test]
async fn drops_oversize_files_and_records_drop_reason() {
    let repo = make_fixture_repo();
    // Add a >10 MB file. We allocate one bytes-buffer so the test
    // stays under a couple seconds even on slow disks.
    let big_path = repo.path().join("oversized.bin");
    let big = vec![b'x'; (cortex_cli::bootstrap::MAX_FILE_BYTES as usize) + 1024];
    fs::write(&big_path, &big).unwrap();
    let cfg = fixture_config();
    let entries = cortex_cli::bootstrap::walk_repo(repo.path(), &cfg);
    assert!(
        entries.iter().any(|e| matches!(
            e,
            cortex_cli::bootstrap::WalkEntry::Dropped {
                reason: "oversize",
                ..
            }
        )),
        "oversize file must surface as Dropped{{reason=oversize}}"
    );
}

#[tokio::test]
async fn since_filter_passes_through_to_git_walker() {
    // We can't drive a real git walker without a git fixture, but we
    // can confirm the runner accepts a `since` and propagates it via
    // the runner config.
    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: Some("HEAD~200".into()),
        dry_run: false,
        kind_filter: Vec::new(),
    };
    let report = run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        Arc::new(Metrics::new()),
        &mut checkpoint,
        None,
        None,
    )
    .await
    .unwrap();
    // Fixture has no `.git`, so the git walker errors and the runner
    // continues without commit events. The file walk still publishes.
    assert!(report.events_published >= 4);
    assert_eq!(report.commits_walked, 0);
}

// ---- phase10e — knowledge + learnings walker ----

#[tokio::test]
async fn walker_emits_knowledge_and_learning_envelopes_from_rulebook() {
    use cortex_cli::bootstrap::run_repo;

    let repo = make_fixture_repo();
    // Add canonical .rulebook/{knowledge,learnings} entries —
    // both top-level and nested so the test pins the
    // recursive-glob behaviour.
    fs::create_dir_all(repo.path().join(".rulebook/knowledge/patterns")).unwrap();
    fs::create_dir_all(repo.path().join(".rulebook/knowledge/anti-patterns")).unwrap();
    fs::create_dir_all(repo.path().join(".rulebook/learnings")).unwrap();
    fs::write(
        repo.path()
            .join(".rulebook/knowledge/patterns/use-rrf-fusion.md"),
        "# Use RRF fusion\n\nReciprocal-rank fusion stabilises lane blends.\n",
    )
    .unwrap();
    fs::write(
        repo.path()
            .join(".rulebook/knowledge/anti-patterns/silently-revert.md"),
        "# Never silently revert\n\nFix forward; uncommitted work is sacred.\n",
    )
    .unwrap();
    fs::write(
        repo.path()
            .join(".rulebook/learnings/2026-04-30-phase10c.md"),
        "# phase10c learning\n\nFile-level body hash is the right dedup key.\n",
    )
    .unwrap();

    let cfg = fixture_config();
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };
    run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        Arc::new(Metrics::new()),
        &mut checkpoint,
        None,
        None,
    )
    .await
    .unwrap();

    let knowledge_envs = publisher.by_kind("knowledge.imported");
    let learning_envs = publisher.by_kind("learning.imported");
    assert_eq!(
        knowledge_envs.len(),
        2,
        "two knowledge files (pattern + anti-pattern) must each emit one envelope"
    );
    assert_eq!(
        learning_envs.len(),
        1,
        "one learning file must emit one envelope"
    );
    // §1.2 — `category` discriminates pattern vs anti-pattern.
    let categories: std::collections::HashSet<String> = knowledge_envs
        .iter()
        .map(|e| {
            e.redacted_payload["category"]
                .as_str()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert!(categories.contains("pattern"));
    assert!(categories.contains("anti-pattern"));
    // §3.2 — body inline; the markdown must round-trip verbatim
    // so the embedder + Meili see the real source content.
    let pattern_body = knowledge_envs
        .iter()
        .find(|e| e.redacted_payload["category"] == "pattern")
        .map(|e| {
            e.redacted_payload["body"]
                .as_str()
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default();
    assert!(
        pattern_body.contains("Reciprocal-rank fusion"),
        "pattern body must contain the markdown verbatim, got {pattern_body:?}"
    );
}

// ---- phase10d — repo casing canonicalization ----

#[test]
fn canonical_repo_lowercases_mixed_case_input() {
    use cortex_cli::bootstrap::canonical_repo;
    assert_eq!(canonical_repo("Cortex"), "cortex");
    assert_eq!(canonical_repo("cortex"), "cortex");
    assert_eq!(canonical_repo("Hive-Hub"), "hive-hub");
    assert_eq!(canonical_repo(""), "");
}

#[tokio::test]
async fn walker_emits_repo_in_canonical_lowercase() {
    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let mut checkpoint = Checkpoint::new("now".into());
    // Mixed-case repo id — the walker MUST lowercase it before
    // stamping `source.repo` so downstream lanes / scope filters
    // resolve `repo: "Cortex"` and `repo: "cortex"` to the same
    // rows.
    let runner_cfg = RunnerConfig {
        repo_id: "Cortex".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };
    run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        Arc::new(Metrics::new()),
        &mut checkpoint,
        None,
        None,
    )
    .await
    .unwrap();
    let envelopes = publisher.snapshot();
    assert!(
        !envelopes.is_empty(),
        "publisher must receive at least one event"
    );
    for (_stream, env) in &envelopes {
        let repo_field = env.source["repo"].as_str().unwrap_or("");
        assert_eq!(
            repo_field, "cortex",
            "every emitted envelope must carry canonical lowercase repo, got {repo_field:?}"
        );
    }
}

// ---- phase10c — bootstrap dedup ledger ----

#[tokio::test]
async fn rerun_with_no_changes_publishes_zero_new_events() {
    use cortex_cli::bootstrap::run_repo_with_dedup;
    use std::sync::{Arc, Mutex};

    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let dedup = Arc::new(Mutex::new(
        cortex_storage::MetadataStore::open_in_memory().unwrap(),
    ));
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };

    // First run — publishes everything, ledger fills up.
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let mut checkpoint = Checkpoint::new("now".into());
    let report1 = run_repo_with_dedup(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        Arc::new(Metrics::new()),
        &mut checkpoint,
        None,
        None,
        Some(dedup.clone()),
    )
    .await
    .unwrap();
    assert!(report1.events_published >= 4);
    assert_eq!(
        report1.files_suppressed, 0,
        "first run has nothing to suppress"
    );
    let ledger_rows = dedup
        .lock()
        .unwrap()
        .bootstrap_seen_count(Some("fixture"))
        .unwrap();
    assert!(
        ledger_rows >= 4,
        "ledger must record at least one row per published file"
    );

    // Second run with the same files — every emit must be
    // suppressed.
    let publisher2 = Arc::new(MemoryPublisher::new());
    let pub_dyn2: Arc<dyn Publisher> = publisher2.clone();
    let mut checkpoint2 = Checkpoint::new("now".into());
    let report2 = run_repo_with_dedup(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn2,
        Arc::new(Metrics::new()),
        &mut checkpoint2,
        None,
        None,
        Some(dedup.clone()),
    )
    .await
    .unwrap();
    // phase26c §3.3 — decision files bypass hash suppression so status
    // changes (pre-phase10i) are picked up on each run. The fixture has
    // one decision file, so a re-run without edits still publishes
    // exactly that one decision event.
    assert_eq!(
        report2.events_published, 1,
        "re-run publishes only the decision re-emit (phase26c §3.3 bypass)"
    );
    assert!(
        report2.files_suppressed > 0,
        "files_suppressed must surface the dedup count for non-decision files"
    );
    assert_eq!(
        publisher2.count(),
        1,
        "publisher receives exactly one event — the decision re-emit"
    );
}

#[tokio::test]
async fn rerun_after_editing_one_file_publishes_only_that_file() {
    use cortex_cli::bootstrap::run_repo_with_dedup;
    use std::sync::{Arc, Mutex};

    let repo = make_fixture_repo();
    let cfg = fixture_config();
    let dedup = Arc::new(Mutex::new(
        cortex_storage::MetadataStore::open_in_memory().unwrap(),
    ));
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };

    // Run #1.
    let mut checkpoint = Checkpoint::new("now".into());
    let publisher1 = Arc::new(MemoryPublisher::new());
    let pub_dyn1: Arc<dyn Publisher> = publisher1.clone();
    run_repo_with_dedup(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn1,
        Arc::new(Metrics::new()),
        &mut checkpoint,
        None,
        None,
        Some(dedup.clone()),
    )
    .await
    .unwrap();

    // Edit one file.
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn hnsw_search(k: usize) -> usize { k * 2 }\n",
    )
    .unwrap();

    // Run #2 — only the edited file publishes.
    let publisher2 = Arc::new(MemoryPublisher::new());
    let pub_dyn2: Arc<dyn Publisher> = publisher2.clone();
    let mut checkpoint2 = Checkpoint::new("now".into());
    let report2 = run_repo_with_dedup(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn2,
        Arc::new(Metrics::new()),
        &mut checkpoint2,
        None,
        None,
        Some(dedup.clone()),
    )
    .await
    .unwrap();
    // phase26c §3.3 — decision files bypass dedup unconditionally, so the
    // fixture's decision file also publishes even without edits. 2 = the
    // edited code file + the decision re-emit.
    assert_eq!(
        report2.events_published, 2,
        "edited code file + decision re-emit (phase26c §3.3)"
    );
    assert!(
        report2.files_suppressed >= 3,
        "every non-decision, non-edited file is suppressed"
    );
    let code_events = publisher2.by_kind("artifact.code");
    assert_eq!(code_events.len(), 1);
    assert_eq!(code_events[0].source["path"], "src/lib.rs");
}

#[test]
fn preflight_flags_lane_inflation_only_when_ledger_empty() {
    use cortex_cli::bootstrap::{preflight_likely_duplicates, PerClassCounts};

    // Empty ledger + lane > 2 × disk → flagged.
    let disk = PerClassCounts {
        decision: 2,
        law: 12,
        analysis: 4,
    };
    let inflated = PerClassCounts {
        decision: 26, // 13×
        law: 37,      // ~3×
        analysis: 33, // ~8×
    };
    let r = preflight_likely_duplicates(true, &disk, &inflated);
    assert!(r.likely_duplicates);
    assert_eq!(r.flagged.len(), 3);

    // Same lane numbers but ledger non-empty → suppressed (a
    // populated ledger means the dedup walker already ran; a
    // separate lane cleanup is the operator's call).
    let r2 = preflight_likely_duplicates(false, &disk, &inflated);
    assert!(!r2.likely_duplicates);
    assert!(r2.flagged.is_empty());

    // Empty ledger but lane within 2× → no flag.
    let healthy = PerClassCounts {
        decision: 3,
        law: 14,
        analysis: 5,
    };
    let r3 = preflight_likely_duplicates(true, &disk, &healthy);
    assert!(!r3.likely_duplicates);
}

#[test]
fn classify_path_via_public_api() {
    let mut cfg = CortexSection::default();
    cfg.decisions.promote_patterns = vec!["docs/decisions/**/*.md".into()];
    assert_eq!(
        cortex_cli::bootstrap::classify_path("docs/decisions/0042/intro.md", &cfg),
        cortex_cli::bootstrap::FileClass::Decision
    );
    assert_eq!(
        cortex_cli::bootstrap::classify_path("src/lib.rs", &cfg),
        cortex_cli::bootstrap::FileClass::Code
    );
    // issue #3 — `.vue` SFCs were silently routed to `FileClass::Other`
    // (which the emitter drops), making the entire Vue layer of any
    // GUI invisible to `cortex_query`. Treat them as Code so the
    // artifact reaches the wire.
    assert_eq!(
        cortex_cli::bootstrap::classify_path("gui/src/App.vue", &cfg),
        cortex_cli::bootstrap::FileClass::Code
    );
}

#[test]
fn dedup_duplicates_helper_groups_paths_with_same_content_hash() {
    // phase10c — synthetic 3× duplicate set. The CLI surface
    // (`cortex-ops bootstrap-dedup --dry-run`) uses
    // `bootstrap_seen_duplicates_by_hash` under the hood; this
    // test exercises that helper directly so the regression
    // signal is local and fast.
    let store = cortex_storage::MetadataStore::open_in_memory().unwrap();
    let now = chrono::Utc::now();
    for path in ["a.md", "b.md", "c.md"] {
        store
            .bootstrap_seen_upsert("R", path, "sha256:dup", None, now)
            .unwrap();
    }
    store
        .bootstrap_seen_upsert("R", "unique.md", "sha256:other", None, now)
        .unwrap();
    let groups = store.bootstrap_seen_duplicates_by_hash("R").unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].count, 3);
    assert_eq!(groups[0].content_hash, "sha256:dup");

    // The helper is repeatable: calling it twice on the same
    // store yields the same answer (idempotent / no mutation).
    let groups2 = store.bootstrap_seen_duplicates_by_hash("R").unwrap();
    assert_eq!(groups, groups2);
}

#[test]
fn estimate_format_string_includes_every_sizing_line() {
    let est = cortex_cli::bootstrap::Estimate {
        repo_id: "Demo".into(),
        files_kept: 10,
        files_dropped: 2,
        code_chunks_est: 100,
        doc_chunks_est: 20,
        commits: 50,
        events_total: 170,
        redacted_bytes_est: 4096,
        classifier_input_tokens_est: 1024,
        classifier_output_tokens_est: 59500,
        embedding_storage_bytes_est: 614400,
        graph_nodes_est: 60,
        graph_edges_est: 108,
        fulltext_index_bytes_est: 510_000,
        runtime_seconds_est: 1,
    };
    let out = cortex_cli::bootstrap::format_estimate(&est);
    for needle in [
        "Repo: Demo",
        "Files (after excludes):",
        "Files dropped:",
        "Code chunks (est):",
        "Doc chunks (est):",
        "Commits:",
        "Est. events:",
        "Est. redacted bytes:",
        "Est. classifier tokens",
        "Est. embedding storage:",
        "Est. graph nodes/edges:",
        "Est. fulltext index:",
        "Est. one-time runtime:",
    ] {
        assert!(out.contains(needle), "estimate missing line: {needle}");
    }
    let _ = Path::new(".");
}
