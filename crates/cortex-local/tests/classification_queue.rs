use cortex_local::{
    classification::{
        queue::{ClassificationQueue, DrainOptions, Job, JobStatus},
        Provider, DEFAULT_MODEL,
    },
    *,
};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    thread,
    time::Duration,
};

fn fixture() -> (tempfile::TempDir, LocalStore, ClassificationQueue) {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    for repository in ["Cortex", "Other"] {
        store
            .capture(CaptureRequest {
                id: Some("subject".into()),
                repository: repository.into(),
                kind: RecordKind::Lesson,
                authority: Authority::Accepted,
                title: "Prefer local records".into(),
                body: "Keep local history; api_key=sk-secret-example-1234567890".into(),
                source: Source {
                    kind: "test".into(),
                    locator: "test:queue".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
    }
    let queue = ClassificationQueue::new(store.clone());
    (root, store, queue)
}

fn options() -> DrainOptions {
    DrainOptions {
        repository: "Cortex".into(),
        max_jobs: 10,
        deadline: Duration::from_secs(5),
    }
}

fn provider(status: u16, delay: Duration) -> (Provider, thread::JoinHandle<()>) {
    provider_with(status, delay, || {})
}

fn provider_with(
    status: u16,
    delay: Duration,
    on_request: impl FnOnce() + Send + 'static,
) -> (Provider, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0; 32768];
        let _ = stream.read(&mut buffer).unwrap();
        on_request();
        thread::sleep(delay);
        let body = json!({"model": DEFAULT_MODEL,"usage":{"input_tokens":17,"output_tokens":3},"answers":{"category":{"type":"choice","choice":"lesson","confidence":0.8,"probabilities":{"decision":0.1,"lesson":0.5,"preference":0.1,"fact":0.1,"transient":0.1,"unknown":0.1}},"durable":{"type":"noul","noul":0.9}}}).to_string();
        let response = format!("HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
        let _ = stream.write_all(response.as_bytes());
    });
    (
        Provider::for_loopback(
            Some("fixture-token".into()),
            Duration::from_secs(5),
            &format!("http://{address}/v1/systemone"),
        )
        .unwrap(),
        handle,
    )
}

fn job_directory(root: &Path, repository: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    root.join("classification/jobs")
        .join(format!("{:x}", Sha256::digest(repository.as_bytes())))
}

fn write_job(root: &Path, job: &Job) {
    std::fs::write(
        job_directory(root, &job.prepared.repository).join(format!("{}.json", job.key)),
        serde_json::to_vec(job).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn durable_cache_survives_restart_reindex_and_crash_before_completion() {
    let (root, store, queue) = fixture();
    let job = queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    let before = store.get("Cortex", "subject").unwrap().unwrap();
    let bytes = std::fs::read_to_string(
        job_directory(root.path(), "Cortex").join(format!("{}.json", job.key)),
    )
    .unwrap();
    assert!(!bytes.contains("sk-secret-example"));
    let (online, handle) = provider(200, Duration::ZERO);
    let report = queue.drain(&online, options()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(report.attempted, 1);
    assert_eq!(report.completed, 1);
    let completed = queue.get("Cortex", &job.key).unwrap().unwrap();
    assert_eq!(completed.known_input_tokens, 17);
    assert_eq!(completed.known_output_tokens, 3);
    assert_eq!(completed.unknown_usage_attempts, 0);
    assert_eq!(store.get("Cortex", "subject").unwrap().unwrap(), before);
    let result = store
        .get("Cortex", &format!("classification:{}", job.key))
        .unwrap()
        .unwrap();
    assert_eq!(result.authority, Authority::Observation);
    assert_eq!(result.links[0].relation, Relation::DerivedFrom);
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("sk-secret-example"));
    write_job(root.path(), &job);
    store.reindex().unwrap();
    let restarted = ClassificationQueue::new(LocalStore::open(root.path()).unwrap());
    let offline = Provider::new(None, Duration::from_secs(1)).unwrap();
    let cached = restarted.drain(&offline, options()).await.unwrap();
    assert_eq!(cached.cached, 1);
    assert_eq!(cached.attempted, 0);
    assert_eq!(
        restarted
            .enqueue("Cortex", "subject", DEFAULT_MODEL)
            .unwrap()
            .status,
        JobStatus::Completed
    );
}

#[tokio::test]
async fn offline_is_pending_without_attempts_and_scope_is_exact() {
    let (_root, _store, queue) = fixture();
    let job = queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    queue.enqueue("Other", "subject", DEFAULT_MODEL).unwrap();
    assert!(queue.get("Other", &job.key).unwrap().is_none());
    assert_eq!(queue.list("Cortex", 10).unwrap().jobs.len(), 1);
    let offline = Provider::new(None, Duration::from_secs(1)).unwrap();
    let result = queue.drain(&offline, options()).await.unwrap();
    assert_eq!(result.pending, 1);
    assert_eq!(result.attempted, 0);
    assert_eq!(queue.get("Cortex", &job.key).unwrap().unwrap().attempts, 0);
    assert!(queue.retry("Other", &job.key).is_err());
}

#[tokio::test]
async fn terminal_failure_retained_and_explicit_retry_preserves_cost_uncertainty() {
    let (_root, _store, queue) = fixture();
    let job = queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    let (online, handle) = provider(401, Duration::ZERO);
    let report = queue.drain(&online, options()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(report.failed, 1);
    let failed = queue.get("Cortex", &job.key).unwrap().unwrap();
    assert_eq!(failed.attempts, 1);
    assert_eq!(failed.unknown_usage_attempts, 1);
    let retried = queue.retry("Cortex", &job.key).unwrap();
    assert_eq!(retried.status, JobStatus::Pending);
    assert_eq!(retried.attempts, 0);
    assert_eq!(retried.unknown_usage_attempts, 1);
    assert!(queue.retry("Cortex", &job.key).is_err());
}

#[tokio::test]
async fn retryable_failure_backoff_lease_and_exhaustion() {
    let (root, _store, queue) = fixture();
    let job = queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    let (online, handle) = provider(503, Duration::ZERO);
    let report = queue.drain(&online, options()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(report.pending, 1);
    let mut pending = queue.get("Cortex", &job.key).unwrap().unwrap();
    assert!(pending.next_attempt_at > chrono::Utc::now().timestamp());
    assert_eq!(queue.drain(&online, options()).await.unwrap().attempted, 0);
    pending.attempts = 5;
    pending.next_attempt_at = 0;
    write_job(root.path(), &pending);
    let exhausted = queue.drain(&online, options()).await.unwrap();
    assert_eq!(exhausted.failed, 1);
    assert_eq!(exhausted.attempted, 0);
}

#[tokio::test]
async fn changed_subject_stales_without_call_and_bad_jobs_are_reported() {
    let (root, store, queue) = fixture();
    let job = queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    let mut value = serde_json::to_value(&job).unwrap();
    value["key"] = json!("bad");
    std::fs::write(
        job_directory(root.path(), "Cortex").join(format!("{}.json", "a".repeat(64))),
        value.to_string(),
    )
    .unwrap();
    std::fs::write(
        job_directory(root.path(), "Cortex").join(format!("{}.json", "b".repeat(64))),
        vec![b' '; 65537],
    )
    .unwrap();
    let record = store.get("Cortex", "subject").unwrap().unwrap();
    store
        .revise(
            "Cortex",
            "subject",
            1,
            RevisionRequest {
                title: record.title,
                body: "changed".into(),
                authority: record.authority,
                links: record.links,
                metadata: record.metadata,
            },
        )
        .unwrap();
    let offline = Provider::new(None, Duration::from_secs(1)).unwrap();
    let report = queue.drain(&offline, options()).await.unwrap();
    assert_eq!(report.stale, 1);
    assert_eq!(report.malformed, 2);
    assert_eq!(report.attempted, 0);
}

#[tokio::test]
async fn drain_bounds_and_total_deadline() {
    let (_root, _store, queue) = fixture();
    queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    let (online, handle) = provider(200, Duration::from_millis(1500));
    let mut opts = options();
    opts.max_jobs = 0;
    assert!(queue.drain(&online, opts.clone()).await.is_err());
    opts.max_jobs = 1;
    opts.deadline = Duration::from_secs(61);
    assert!(queue.drain(&online, opts.clone()).await.is_err());
    opts.deadline = Duration::from_secs(1);
    let report = queue.drain(&online, opts).await.unwrap();
    handle.join().unwrap();
    assert!(report.deadline_exhausted);
    assert_eq!(report.pending, 1);
    assert_eq!(report.attempted, 1);
}

#[tokio::test]
async fn revision_during_http_preserves_original_advice_and_store_remains_available() {
    let (_root, store, queue) = fixture();
    let job = queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    let editing = store.clone();
    let (online, handle) = provider_with(200, Duration::ZERO, move || {
        let record = editing.get("Cortex", "subject").unwrap().unwrap();
        editing
            .revise(
                "Cortex",
                "subject",
                1,
                RevisionRequest {
                    title: record.title,
                    body: "new revision while provider runs".into(),
                    authority: Authority::Accepted,
                    links: record.links,
                    metadata: record.metadata,
                },
            )
            .unwrap();
    });
    assert_eq!(queue.drain(&online, options()).await.unwrap().stale, 1);
    handle.join().unwrap();
    assert_eq!(store.get("Cortex", "subject").unwrap().unwrap().revision, 2);
    let result = store
        .get("Cortex", &format!("classification:{}", job.key))
        .unwrap()
        .unwrap();
    assert_eq!(result.metadata["prepared"]["revision"], 1);
    assert_eq!(
        result.metadata["prepared"]["content_sha256"],
        job.prepared.content_sha256
    );
}

#[tokio::test]
async fn max_jobs_bounds_requests_and_reports_unprocessed_jobs() {
    let (_root, store, queue) = fixture();
    queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    store
        .capture(CaptureRequest {
            id: Some("second".into()),
            repository: "Cortex".into(),
            body: "second".into(),
            source: Source {
                kind: "test".into(),
                locator: "test:second".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    queue.enqueue("Cortex", "second", DEFAULT_MODEL).unwrap();
    assert!(queue.list("Cortex", 1).unwrap().incomplete);
    let (online, handle) = provider(200, Duration::ZERO);
    let mut opts = options();
    opts.max_jobs = 1;
    let report = queue.drain(&online, opts).await.unwrap();
    handle.join().unwrap();
    assert_eq!(report.completed, 1);
    assert_eq!(report.pending, 1);
    assert_eq!(report.attempted, 1);
}

#[test]
fn admission_ceiling_is_explicit_and_foreign_queue_cannot_starve_or_leak() {
    let (root, _store, queue) = fixture();
    let existing = queue.enqueue("Other", "subject", DEFAULT_MODEL).unwrap();
    let directory = job_directory(root.path(), "Other");
    for index in 0..9999 {
        std::fs::write(directory.join(format!("invalid-{index}.json")), b"!").unwrap();
    }
    let foreign = queue.list("Other", 10000).unwrap();
    assert_eq!(foreign.scanned, 10000);
    assert_eq!(foreign.malformed, 9999);
    assert!(!foreign.incomplete);
    assert_eq!(
        queue
            .enqueue("Other", "subject", DEFAULT_MODEL)
            .unwrap()
            .key,
        existing.key
    );
    assert!(queue.enqueue("Other", "subject", "jev-latest").is_err());
    queue.enqueue("Cortex", "subject", DEFAULT_MODEL).unwrap();
    let own = queue.list("Cortex", 10000).unwrap();
    assert_eq!(own.scanned, 1);
    assert_eq!(own.malformed, 0);
    assert!(!own.incomplete);
}
