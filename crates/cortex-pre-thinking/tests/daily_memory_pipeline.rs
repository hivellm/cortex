//! Mandatory hermetic local-memory pipeline contracts; no external services.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortex_api::{QueryRequest, QueryResponse};
use cortex_pre_thinking::breaker::{Breaker, BreakerConfig};
use cortex_pre_thinking::daily_memory::{
    project_query, MEMORY_BYTES, MEMORY_PREFIX, MEMORY_SUFFIX,
};
use cortex_pre_thinking::{
    run_with_breaker_and_memory, run_with_memory, DailyMemoryContext, DailyMemoryFn,
    DailyMemoryRequest, Metrics, PreThinkingBudget, PreThinkingInput, QueryFn,
};
use serde_json::json;

struct Memory {
    requests: Mutex<Vec<DailyMemoryRequest>>,
    delay: Duration,
    extra: usize,
}

impl Memory {
    fn new(delay: Duration) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            delay,
            extra: 0,
        }
    }
}

#[async_trait]
impl DailyMemoryFn for Memory {
    async fn read(&self, request: DailyMemoryRequest) -> Option<DailyMemoryContext> {
        self.requests.lock().unwrap().push(request.clone());
        tokio::time::sleep(self.delay).await;
        let context = if self.extra == 0 {
            "[memory ADR-42 v1 event original-source] source=src/store.rs\nKeep SQLite correction commits durable.\n".to_owned()
        } else {
            "x".repeat(request.budget_bytes + self.extra)
        };
        Some(DailyMemoryContext {
            context,
            memories: vec![
                json!({"id":"ADR-42","version":1,"event_id":"original-source","scope":{"repository":request.repo}}),
            ],
            revision: 7,
            global_revision: None,
            delivery_id: "prepared-local-1".into(),
            stage: "prepared".into(),
            cache_hit: true,
        })
    }
}

struct Optional {
    requests: Mutex<Vec<QueryRequest>>,
    delay: Duration,
}

#[async_trait]
impl QueryFn for Optional {
    async fn query(&self, request: QueryRequest) -> Option<QueryResponse> {
        self.requests.lock().unwrap().push(request);
        tokio::time::sleep(self.delay).await;
        None
    }
}

fn input<'a>(cwd: &'a Path, prompt: &'a str) -> PreThinkingInput<'a> {
    PreThinkingInput {
        session_id: "observed-session",
        turn_id: "observed-turn",
        user_prompt: prompt,
        cwd,
        recent_files: &[],
        budget: PreThinkingBudget {
            bundle_bytes: 4096,
            time_ms: 600,
        },
        principal: None,
    }
}

fn repo() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join(".git")).unwrap();
    std::fs::write(
        directory.path().join(".git/HEAD"),
        "ref: refs/heads/feature/memory\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("cortex.toml"),
        "[cortex]\nid = \"Exact/Repo\"\n",
    )
    .unwrap();
    directory
}

#[tokio::test]
async fn same_basename_roots_require_explicit_identity_before_reading_daily_memory() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("a/app");
    let second = directory.path().join("b/app");
    for root in [&first, &second] {
        std::fs::create_dir_all(root.join(".git")).unwrap();
    }
    // This represents an accepted pinned response in the shared `app` scope.
    // Neither basename alone establishes that the checkout owns that scope.
    let memory = Arc::new(Memory::new(Duration::ZERO));
    let optional = Arc::new(Optional {
        requests: Mutex::new(Vec::new()),
        delay: Duration::ZERO,
    });
    for root in [&first, &second] {
        let output = run_with_memory(
            &input(root, "memory"),
            optional.clone(),
            memory.clone(),
            Arc::new(Metrics::new()),
        )
        .await;
        assert_eq!(output.memory_outcome, "scope_identity_required");
        assert!(output.fail_open);
        assert!(output.bundle.is_empty());
        assert!(output.memory_delivery_id.is_none());
    }
    assert!(memory.requests.lock().unwrap().is_empty());
    // Optional full-stack lookup retains its existing basename contract.
    assert_eq!(optional.requests.lock().unwrap().len(), 2);
    assert!(optional
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.scope.repo.as_deref() == Some("app")));

    std::fs::write(first.join("cortex.toml"), "[cortex]\nid='app'\n").unwrap();
    let output = cortex_pre_thinking::pipeline::run_memory_only(
        &input(&first, ""),
        memory.clone(),
        Arc::new(Metrics::new()),
    )
    .await;
    assert_eq!(output.memory_outcome, "prepared");
    assert!(output.bundle.contains("original-source"));
    let output = cortex_pre_thinking::pipeline::run_memory_only(
        &input(&second, ""),
        memory.clone(),
        Arc::new(Metrics::new()),
    )
    .await;
    assert_eq!(output.memory_outcome, "scope_identity_required");
    assert!(output.bundle.is_empty());
    let requests = memory.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].repo, "app");
}

#[test]
fn daily_identity_comes_only_from_valid_configuration_at_nearest_git_root() {
    let directory = repo();
    let nested = directory.path().join("nested/project");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(
        cortex_pre_thinking::scope::derive("", &nested, &[])
            .memory_repo
            .as_deref(),
        Some("Exact/Repo")
    );
    std::fs::create_dir(nested.join(".git")).unwrap();
    // A nested Git root must not borrow its parent's explicit identity.
    assert!(cortex_pre_thinking::scope::derive("", &nested, &[])
        .memory_repo
        .is_none());
    for config in [
        "[cortex]\nid=' padded '\n".to_owned(),
        "[cortex]\nid=42\n".to_owned(),
        "[cortex]\nid='broken\n".to_owned(),
        format!("[cortex]\nid='{}'\n", "x".repeat(257)),
        format!("[cortex]\nid='app'\n#{}", "x".repeat(65536)),
    ] {
        std::fs::write(nested.join("cortex.toml"), config).unwrap();
        let derived = cortex_pre_thinking::scope::derive("", &nested, &[]);
        assert_eq!(derived.scope.repo.as_deref(), Some("project"));
        assert!(derived.memory_repo.is_none());
    }
    std::fs::write(nested.join("cortex.toml"), "[cortex]\nid='Nested/Exact'\n").unwrap();
    assert_eq!(
        cortex_pre_thinking::scope::derive("", &nested, &[])
            .memory_repo
            .as_deref(),
        Some("Nested/Exact")
    );
}

#[tokio::test]
async fn relative_cwd_cannot_authorize_daily_memory_even_with_explicit_id() {
    // A relative path must actually resolve a valid config to catch accidental
    // authorization through the process CWD; never mutate global process CWD.
    let cwd = std::env::current_dir().unwrap();
    let directory = tempfile::tempdir_in(&cwd).unwrap();
    std::fs::create_dir(directory.path().join(".git")).unwrap();
    std::fs::write(
        directory.path().join("cortex.toml"),
        "[cortex]\nid='Relative/Explicit'\n",
    )
    .unwrap();
    let relative = directory.path().strip_prefix(&cwd).unwrap();
    assert!(!relative.is_absolute());
    let derived = cortex_pre_thinking::scope::derive("", relative, &[]);
    assert_eq!(derived.scope.repo.as_deref(), Some("Relative/Explicit"));
    assert!(derived.memory_repo.is_none());
    let absolute = cortex_pre_thinking::scope::derive("", directory.path(), &[]);
    assert_eq!(absolute.memory_repo.as_deref(), Some("Relative/Explicit"));

    let memory = Arc::new(Memory::new(Duration::ZERO));
    let optional = Arc::new(Optional {
        requests: Mutex::new(Vec::new()),
        delay: Duration::ZERO,
    });
    let output = run_with_memory(
        &input(relative, "memory"),
        optional.clone(),
        memory.clone(),
        Arc::new(Metrics::new()),
    )
    .await;
    assert_eq!(output.memory_outcome, "scope_identity_required");
    assert!(output.bundle.is_empty());
    assert!(memory.requests.lock().unwrap().is_empty());
    let requests = optional.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].scope.repo.as_deref(), Some("Relative/Explicit"));
}

#[tokio::test]
async fn cited_memory_survives_optional_outage_and_has_separate_receipt() {
    let directory = repo();
    let memory = Arc::new(Memory::new(Duration::ZERO));
    let optional = Arc::new(Optional {
        requests: Mutex::new(Vec::new()),
        delay: Duration::ZERO,
    });
    let output = run_with_memory(
        &input(directory.path(), "fix src/store.rs"),
        optional.clone(),
        memory.clone(),
        Arc::new(Metrics::new()),
    )
    .await;
    assert!(output.fail_open);
    assert!(output.bundle.starts_with(MEMORY_PREFIX));
    assert!(output.bundle.contains("original-source"));
    assert!(output
        .bundle
        .contains("Keep SQLite correction commits durable."));
    assert!(!output.bundle.contains("reason=internal"));
    assert_eq!(
        output.memory_delivery_id.as_deref(),
        Some("prepared-local-1")
    );
    assert_eq!(output.query_id, None);
    assert_eq!(output.memory_revision, Some(7));
    assert!(output.memory_cache_hit);
    let requests = memory.requests.lock().unwrap();
    assert_eq!(requests[0].repo, "Exact/Repo");
    assert_eq!(requests[0].branch.as_deref(), Some("feature/memory"));
    assert_eq!(requests[0].paths, vec!["src/store.rs"]);
    assert!(!requests[0].include_global);
    assert_eq!(
        requests[0].budget_bytes + MEMORY_PREFIX.len() + MEMORY_SUFFIX.len(),
        MEMORY_BYTES
    );
    assert_eq!(optional.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn open_optional_breaker_still_reads_local_memory_before_guard() {
    let directory = repo();
    let breaker = Arc::new(Breaker::with_config(BreakerConfig {
        threshold: 1,
        window: Duration::from_secs(60),
        cooldown: Duration::from_secs(60),
    }));
    let optional = Arc::new(Optional {
        requests: Mutex::new(Vec::new()),
        delay: Duration::ZERO,
    });
    let memory = Arc::new(Memory::new(Duration::ZERO));
    for _ in 0..2 {
        let output = run_with_breaker_and_memory(
            &input(directory.path(), "memory"),
            optional.clone(),
            memory.clone(),
            Arc::new(Metrics::new()),
            breaker.clone(),
        )
        .await;
        assert!(output.bundle.contains("original-source"));
    }
    assert_eq!(optional.requests.lock().unwrap().len(), 1);
    assert_eq!(memory.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn local_first_and_optional_share_a_clamped_600ms_total_deadline() {
    let directory = repo();
    let memory = Arc::new(Memory::new(Duration::from_millis(180)));
    let optional = Arc::new(Optional {
        requests: Mutex::new(Vec::new()),
        delay: Duration::from_secs(5),
    });
    let mut request = input(directory.path(), "memory");
    request.budget.time_ms = 5000;
    let started = Instant::now();
    let output =
        run_with_memory(&request, optional.clone(), memory, Arc::new(Metrics::new())).await;
    assert!(
        started.elapsed() < Duration::from_millis(750),
        "deadline exceeded: {:?}",
        started.elapsed()
    );
    assert!(output.bundle.contains("original-source"));
    assert!(!output.bundle.contains("reason=timeout"));
    assert!(optional.requests.lock().unwrap()[0].budget_ms <= 430);
}

#[tokio::test]
async fn unresolved_scope_skips_memory_and_invalid_context_is_never_trimmed_after_receipt() {
    let directory = tempfile::tempdir().unwrap();
    // Temp directories can themselves sit below a checkout on developer hosts.
    let unresolved_root = directory.path().ancestors().last().unwrap();
    assert!(cortex_pre_thinking::scope::repo_from_cwd(unresolved_root).is_none());
    let memory = Arc::new(Memory::new(Duration::ZERO));
    let optional = Arc::new(Optional {
        requests: Mutex::new(Vec::new()),
        delay: Duration::ZERO,
    });
    let output = run_with_memory(
        &input(unresolved_root, "memory"),
        optional.clone(),
        memory.clone(),
        Arc::new(Metrics::new()),
    )
    .await;
    assert_eq!(output.memory_outcome, "scope_unresolved");
    assert!(memory.requests.lock().unwrap().is_empty());
    // Completed unresolved derivation retains the existing optional-query
    // contract. An I/O timeout must never manufacture this same broad scope.
    assert_eq!(optional.requests.lock().unwrap().len(), 1);
    assert_eq!(optional.requests.lock().unwrap()[0].scope.repo, None);
    let directory = repo();
    let oversized = Arc::new(Memory {
        requests: Mutex::new(Vec::new()),
        delay: Duration::ZERO,
        extra: 1,
    });
    let output = run_with_memory(
        &input(directory.path(), "memory"),
        optional,
        oversized,
        Arc::new(Metrics::new()),
    )
    .await;
    assert_eq!(output.memory_outcome, "invalid_response");
    assert_eq!(output.memory_delivery_id, None);
    assert!(!output.bundle.contains(MEMORY_PREFIX));
}

#[test]
fn projected_query_keeps_unicode_marks_and_at_most_32_distinct_literal_terms() {
    let prompt = format!(
        "ac\u{0327}a\u{0303}o OR NEAR {}",
        (0..100)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let query = project_query(&prompt);
    assert!(query.starts_with("ac\u{0327}a\u{0303}o OR NEAR"));
    assert_eq!(query.split_whitespace().count(), 32);
    assert!(query.len() <= 4096);
    assert!(project_query(&"界".repeat(3000)).len() <= 4096);
}

#[test]
fn branch_worktree_metadata_and_strict_paths_do_not_infer_external_scope() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("metadata")).unwrap();
    std::fs::write(directory.path().join(".git"), "gitdir: metadata\n").unwrap();
    std::fs::write(
        directory.path().join("metadata/HEAD"),
        "ref: refs/heads/worktree\n",
    )
    .unwrap();
    assert_eq!(
        cortex_pre_thinking::scope::branch_from_cwd(directory.path()).as_deref(),
        Some("worktree")
    );
    std::fs::write(directory.path().join("metadata/HEAD"), "deadbeef\n").unwrap();
    assert_eq!(
        cortex_pre_thinking::scope::branch_from_cwd(directory.path()),
        None
    );
    assert_eq!(
        cortex_pre_thinking::scope::memory_paths(&[
            "../secrets.env".into(),
            "/outside/file.rs".into(),
            "C:\\outside\\file.rs".into(),
            "src\\store.rs".into()
        ]),
        vec!["src/store.rs"]
    );
    assert!(
        cortex_pre_thinking::extract_prompt_files("inspect /outside/file.rs and ../secret.rs")
            .is_empty()
    );
}

#[test]
fn scope_timeout_does_not_issue_memory_or_unscoped_optional_queries() {
    let directory = repo();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let blocker = runtime.spawn_blocking(move || {
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let memory = Arc::new(Memory::new(Duration::ZERO));
    let optional = Arc::new(Optional {
        requests: Mutex::new(Vec::new()),
        delay: Duration::ZERO,
    });
    let mut request = input(directory.path(), "memory");
    request.budget.time_ms = 40;
    let output = runtime.block_on(run_with_memory(
        &request,
        optional.clone(),
        memory.clone(),
        Arc::new(Metrics::new()),
    ));
    // Release the worker before assertions, including when a regression fails.
    drop(release_tx);
    runtime.block_on(blocker).unwrap();
    assert!(output.fail_open);
    assert_eq!(output.memory_outcome, "scope_timeout");
    assert!(output.bundle.is_empty());
    assert!(memory.requests.lock().unwrap().is_empty());
    assert!(optional.requests.lock().unwrap().is_empty());
}
