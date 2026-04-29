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
    assert!(est.files_kept >= 4, "fixture must keep code+doc+adr+law+memory");
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

    assert_eq!(report.repo_id, "Fixture");
    assert!(report.events_published >= 4);

    // Spec-09 promotions must produce one event each.
    assert_eq!(publisher.by_kind("decision.imported").len(), 1);
    assert_eq!(publisher.by_kind("law.imported").len(), 1);
    assert_eq!(publisher.by_kind("memory.imported").len(), 1);
    assert!(!publisher.by_kind("artifact.code").is_empty());
    assert!(!publisher.by_kind("artifact.doc").is_empty());

    // Checkpoint marks the repo done.
    assert!(checkpoint.is_repo_done("Fixture"));
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
        .get("Fixture")
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

    assert!(publisher2.count() <= 1, "resume past last_file emits at most stragglers");
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
    let body = env_doc.redacted_payload["body"]
        .as_str()
        .expect("body str");
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
    assert!(reloaded.is_repo_done("Fixture"));
    let progress = reloaded.repos.get("Fixture").unwrap();
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
            cortex_cli::bootstrap::WalkEntry::Dropped { reason: "oversize", .. }
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
