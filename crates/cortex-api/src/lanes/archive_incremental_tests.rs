use super::*;

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;

use cortex_core::events::{Context, Envelope, Kind, Stream, Turn};

use crate::lanes::{MemoryGraphLane, MemoryVectorLane};
use crate::orchestrator::Orchestrator;
use crate::types::{IncludeField, Intent, QueryRequest, QueryResponse, Scope};
use crate::{QueryService, ServiceOutcome};

struct ArchiveFixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    metadata: PathBuf,
    source: PathBuf,
}

impl ArchiveFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("archive");
        let partition = root.join("events/year=2026/month=10/day=16/hour=12");
        fs::create_dir_all(&partition).unwrap();
        let source = partition.join("events-000000.parquet");
        let metadata = temp.path().join("metadata.sqlite");
        Self {
            _temp: temp,
            root,
            metadata,
            source,
        }
    }

    fn open(&self) -> ArchiveIndex {
        ArchiveIndex::open(&self.root, &self.metadata).unwrap()
    }
}

fn turn(event_id: &str, message: &str) -> Envelope {
    Envelope {
        event_id: event_id.to_string(),
        schema_version: "1".to_string(),
        occurred_at: "2026-10-16T12:00:00.000Z".to_string(),
        ingested_at: None,
        session_id: "01TURN0SESSION00000000000Z".to_string(),
        stream: Stream::Live,
        tool: "claude-code".to_string(),
        model: None,
        kind: Kind::Turn,
        context: Context {
            repo: Some("Cortex".to_string()),
            branch: None,
            commit: None,
            cwd: Some("e:/HiveLLM/Cortex".to_string()),
            user: None,
            platform: "win32".to_string(),
            ide: Some("claude-code".to_string()),
            extras: BTreeMap::new(),
        },
        payload: serde_json::to_value(Turn {
            user_message: message.to_string(),
            assistant_message: None,
            tokens: None,
            tool_call_event_ids: Vec::new(),
        })
        .unwrap(),
        redactions: Vec::new(),
        content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        parent_event_id: None,
        class_level: None,
        class_compartments: None,
    }
}

/// The production writer commits one complete zstd frame per envelope.
fn append_frame(path: &Path, envelope: &Envelope) {
    let mut line = serde_json::to_vec(envelope).unwrap();
    line.push(b'\n');
    let frame = zstd::encode_all(line.as_slice(), 1).unwrap();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(&frame).unwrap();
    file.sync_all().unwrap();
}

fn snapshot_ids(lane: &MemoryKeywordLane) -> Vec<String> {
    let snapshots = lane.hits.lock().unwrap();
    let mut ids: Vec<_> = snapshots
        .get(INDEXES[0])
        .into_iter()
        .flatten()
        .map(|hit| hit.doc_id.clone())
        .collect();
    ids.sort();
    ids
}

fn settle(
    index: &mut ArchiveIndex,
    lane: &MemoryKeywordLane,
    metrics: &LoaderMetrics,
    expected_hits: usize,
) -> ArchivePollReport {
    for _ in 0..64 {
        poll_and_publish(index, lane, metrics).unwrap();
        let status = lane.archive_status.lock().unwrap().clone();
        let report = status.last_poll.unwrap();
        assert!(report.errors.is_empty(), "unexpected errors: {report:?}");
        if report.discovery_complete
            && !report.historical_backfill
            && report.pending_files == 0
            && report.partial_files == 0
            && status.snapshot_hits == expected_hits
        {
            return report;
        }
    }
    panic!(
        "archive did not settle: {:?}",
        lane.archive_status.lock().unwrap()
    );
}

#[test]
fn append_deduplicates_and_restart_restores_without_decoding_closed_history() {
    let fixture = ArchiveFixture::new();
    let first = turn("archive-first", "first archive marker");
    let second = turn("archive-second", "second archive marker");
    append_frame(&fixture.source, &first);

    let lane = MemoryKeywordLane::new();
    let metrics = LoaderMetrics::new();
    let meili_hit = envelope_to_hit(&turn("meili-only", "live Meili marker")).unwrap();
    lane.seed("cortex-meili-turns", vec![meili_hit.clone()]);
    let mut index = fixture.open();
    settle(&mut index, &lane, &metrics, 1);
    assert!(metrics.archive_bytes_read.load(Ordering::Relaxed) > 0);
    assert!(metrics.archive_bytes_decoded.load(Ordering::Relaxed) > 0);

    let bytes_read = metrics.archive_bytes_read.load(Ordering::Relaxed);
    let bytes_decoded = metrics.archive_bytes_decoded.load(Ordering::Relaxed);
    let revision = lane.archive_revision.load(Ordering::Acquire);
    let idle = settle(&mut index, &lane, &metrics, 1);
    assert_eq!((idle.bytes_read, idle.bytes_decoded), (0, 0));
    assert_eq!(
        metrics.archive_bytes_read.load(Ordering::Relaxed),
        bytes_read
    );
    assert_eq!(
        metrics.archive_bytes_decoded.load(Ordering::Relaxed),
        bytes_decoded
    );
    assert_eq!(lane.archive_revision.load(Ordering::Acquire), revision);

    append_frame(&fixture.source, &first);
    append_frame(&fixture.source, &second);
    settle(&mut index, &lane, &metrics, 2);
    let expected = vec![
        "archive|archive-first".to_string(),
        "archive|archive-second".to_string(),
    ];
    assert_eq!(snapshot_ids(&lane), expected);
    assert!(lane.archive_revision.load(Ordering::Acquire) > revision);
    {
        let snapshots = lane.hits.lock().unwrap();
        for alias in INDEXES {
            assert_eq!(snapshots.get(alias), snapshots.get(INDEXES[0]));
        }
        assert_eq!(snapshots["cortex-meili-turns"], vec![meili_hit]);
    }
    drop(index);

    let reopened_lane = MemoryKeywordLane::new();
    let reopened_metrics = LoaderMetrics::new();
    let report = settle(&mut fixture.open(), &reopened_lane, &reopened_metrics, 2);
    assert_eq!(snapshot_ids(&reopened_lane), expected);
    assert_eq!((report.bytes_read, report.bytes_decoded), (0, 0));
    assert_eq!(
        reopened_metrics
            .archive_bytes_decoded
            .load(Ordering::Relaxed),
        0
    );
    let status = reopened_lane.archive_status.lock().unwrap();
    assert_eq!(status.phase, "tailing");
    assert_eq!(status.snapshot_hits, 2);
    assert!(status.last_error.is_none());
}

#[test]
fn forget_removes_lane_hits_and_suppresses_new_file_replay_after_reopen() {
    let fixture = ArchiveFixture::new();
    let forgotten = turn("forgotten-event", "private archive marker");
    let survivor = turn("surviving-event", "retained archive marker");
    append_frame(&fixture.source, &forgotten);
    append_frame(&fixture.source, &survivor);
    let source_bytes = fs::read(&fixture.source).unwrap();
    let lane = MemoryKeywordLane::new();
    let metrics = LoaderMetrics::new();
    let unrelated = envelope_to_hit(&turn("meili-survivor", "independent Meili hit")).unwrap();
    lane.seed("cortex-meili-decisions", vec![unrelated.clone()]);
    let mut index = fixture.open();
    settle(&mut index, &lane, &metrics, 2);
    let revision = lane.archive_revision.load(Ordering::Acquire);

    forget_projection(&fixture.metadata, &forgotten.event_id, Some(&lane)).unwrap();
    assert_eq!(snapshot_ids(&lane), vec!["archive|surviving-event"]);
    assert!(lane.archive_revision.load(Ordering::Acquire) > revision);
    assert_eq!(lane.archive_status.lock().unwrap().snapshot_hits, 1);
    assert!(lane
        .hits
        .lock()
        .unwrap()
        .values()
        .flatten()
        .all(|hit| { hit.doc_id != "archive|forgotten-event" && hit.doc_id != "forgotten-event" }));

    append_frame(
        &fixture.source.with_file_name("replayed.parquet"),
        &forgotten,
    );
    settle(&mut index, &lane, &metrics, 1);
    assert_eq!(snapshot_ids(&lane), vec!["archive|surviving-event"]);
    assert_eq!(
        lane.hits.lock().unwrap()["cortex-meili-decisions"],
        vec![unrelated]
    );
    assert_eq!(fs::read(&fixture.source).unwrap(), source_bytes);
    drop(index);

    let reopened_lane = MemoryKeywordLane::new();
    settle(
        &mut fixture.open(),
        &reopened_lane,
        &LoaderMetrics::new(),
        1,
    );
    assert_eq!(
        snapshot_ids(&reopened_lane),
        vec!["archive|surviving-event"]
    );
}

async fn query(service: &QueryService, request: QueryRequest) -> Box<QueryResponse> {
    match service.handle("archive-integration-test", request).await {
        ServiceOutcome::Ok(response) => response,
        other => panic!("expected successful archive query, got {other:?}"),
    }
}

fn snippet_ids(response: &QueryResponse) -> Vec<&str> {
    let mut ids: Vec<_> = response
        .results
        .snippets
        .iter()
        .filter_map(|snippet| snippet.doc_id.as_deref())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn archive_reprojection_enforces_classification_and_exact_repository_for_real_keys() {
    use crate::storage::api_keys::ApiKeyStore;
    use crate::{Principal, PrincipalStore, RoleBinding};
    use axum::http::{HeaderMap, HeaderValue};
    use std::sync::RwLock;

    let fixture = ArchiveFixture::new();
    for (id, repo, level, compartments) in [
        ("public", "foo-bar", 0, Vec::<String>::new()),
        ("secret", "foo-bar", 2, vec!["financial".to_owned()]),
        ("compartment", "foo-bar", 0, vec!["financial".to_owned()]),
        ("collision", "foo_bar", 0, Vec::new()),
    ] {
        let mut envelope = turn(id, "archive access marker");
        envelope.context.repo = Some(repo.to_owned());
        envelope.class_level = Some(level);
        envelope.class_compartments = Some(compartments);
        append_frame(&fixture.source, &envelope);
    }
    // Simulate an existing database written by the old projection contract.
    let mut old = fixture.open();
    for _ in 0..100 {
        old.poll(|envelope| {
            let mut hit = envelope_to_hit(envelope)?;
            hit.extras.remove("class_level");
            hit.extras.remove("class_compartments");
            serde_json::to_value(hit).ok()
        })
        .unwrap();
        if old.snapshot().unwrap().len() == 4 {
            break;
        }
    }
    assert_eq!(old.snapshot().unwrap().len(), 4);
    drop(old);

    let mut index = fixture.open();
    index.require_projection_version("lane-hit-acl-v1").unwrap();
    assert!(
        index.snapshot().unwrap().is_empty(),
        "old unlabeled hits must be hidden immediately"
    );
    let lane = Arc::new(MemoryKeywordLane::new());
    settle(&mut index, &lane, &LoaderMetrics::new(), 4);

    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let public = keys
        .issue_with_role("api", "public", Some("public"))
        .unwrap();
    let high = keys.issue_with_role("api", "high", Some("high")).unwrap();
    let permitted = keys
        .issue_with_role("api", "permitted", Some("permitted"))
        .unwrap();
    let principals = PrincipalStore::new(
        [
            (
                "public".into(),
                RoleBinding {
                    clearance_level: 0,
                    compartments: vec![],
                },
            ),
            (
                "high".into(),
                RoleBinding {
                    clearance_level: 2,
                    compartments: vec![],
                },
            ),
            (
                "permitted".into(),
                RoleBinding {
                    clearance_level: 2,
                    compartments: vec!["financial".into()],
                },
            ),
        ],
        Principal::public_only("default"),
    );
    let service = QueryService::with_memory_defaults(
        Orchestrator::new(
            Arc::new(MemoryVectorLane::new()),
            lane.clone(),
            Arc::new(MemoryGraphLane::new()),
        )
        .with_access_control(cortex_config::AccessControlConfig {
            enabled: true,
            ..Default::default()
        }),
    )
    .with_indexed_repos(lane)
    .with_principal_store(Arc::new(RwLock::new(principals)))
    .with_api_key_store(keys);
    let request = QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope {
            repo: Some("foo-bar".into()),
            ..Scope::default()
        },
        query: "archive access marker".into(),
        limit: 20,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 2000,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,
        principal: None,
    };
    for (key, expected) in [
        (&public, vec!["archive|public"]),
        (&high, vec!["archive|public"]),
        (
            &permitted,
            vec!["archive|compartment", "archive|public", "archive|secret"],
        ),
    ] {
        service.acl.set_allowed(&key.id, vec!["foo-bar".into()]);
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {}", key.cleartext)).unwrap(),
        );
        for _ in 0..2 {
            let ServiceOutcome::Ok(response) = service
                .handle_with_headers("adapter", request.clone(), &headers)
                .await
            else {
                panic!("authorized archive query must succeed");
            };
            assert_eq!(snippet_ids(&response), expected);
        }
    }
    drop(index);
    let mut reopened = fixture.open();
    reopened
        .require_projection_version("lane-hit-acl-v1")
        .unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().len(),
        4,
        "same contract must not restart backfill"
    );
}

#[tokio::test]
async fn archive_publication_and_forget_invalidate_query_service_cached_response() {
    let fixture = ArchiveFixture::new();
    let first = turn("cached-first", "cache archive marker");
    append_frame(&fixture.source, &first);
    append_frame(
        &fixture.source,
        &turn("irrelevant-event", "unrelated database compaction"),
    );
    let mut other_repo = turn("other-repo-event", "cache archive marker");
    other_repo.context.repo = Some("Vectorizer".to_string());
    append_frame(&fixture.source, &other_repo);
    let lane = Arc::new(MemoryKeywordLane::new());
    let metrics = LoaderMetrics::new();
    let mut index = fixture.open();
    settle(&mut index, &lane, &metrics, 3);
    let service = QueryService::with_memory_defaults(Orchestrator::new(
        Arc::new(MemoryVectorLane::new()),
        lane.clone(),
        Arc::new(MemoryGraphLane::new()),
    ))
    .with_indexed_repos(lane.clone());
    let request = QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope {
            repo: Some("Cortex".to_string()),
            ..Scope::default()
        },
        query: "cache archive marker".to_string(),
        limit: 20,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 500,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,
        principal: None,
    };
    let original = query(&service, request.clone()).await;
    assert_eq!(original.budget.cache, "miss");
    assert_eq!(snippet_ids(&original), vec!["archive|cached-first"]);
    let cached = query(&service, request.clone()).await;
    assert_eq!(cached.budget.cache, "hit");
    assert_eq!(cached.query_id, original.query_id);

    append_frame(
        &fixture.source,
        &turn("cached-second", "new cache archive marker"),
    );
    settle(&mut index, &lane, &metrics, 4);
    let appended = query(&service, request.clone()).await;
    assert_eq!(appended.budget.cache, "miss");
    assert_ne!(appended.query_id, original.query_id);
    assert_eq!(
        snippet_ids(&appended),
        vec!["archive|cached-first", "archive|cached-second"]
    );
    assert_eq!(query(&service, request.clone()).await.budget.cache, "hit");

    // Do not clear the cache: the archive revision itself must prevent stale reuse.
    forget_projection(&fixture.metadata, &first.event_id, Some(&lane)).unwrap();
    let forgotten = query(&service, request).await;
    assert_eq!(forgotten.budget.cache, "miss");
    assert_ne!(forgotten.query_id, appended.query_id);
    assert_eq!(snippet_ids(&forgotten), vec!["archive|cached-second"]);
}

#[test]
fn status_distinguishes_disabled_configuration_and_corrupt_source_failures() {
    let fixture = ArchiveFixture::new();
    let lane = Arc::new(MemoryKeywordLane::new());
    let metrics = Arc::new(LoaderMetrics::new());
    start(
        false,
        Some(fixture.root.clone()),
        fixture.metadata.clone(),
        lane.clone(),
        metrics.clone(),
        Duration::from_secs(1),
    );
    {
        let status = lane.archive_status.lock().unwrap();
        assert!(!status.enabled);
        assert_eq!(status.phase, "disabled");
        assert!(status.last_poll.is_none());
        assert!(!fixture.metadata.exists());
    }
    start(
        true,
        None,
        fixture.metadata.clone(),
        lane.clone(),
        metrics.clone(),
        Duration::from_secs(1),
    );
    {
        let status = lane.archive_status.lock().unwrap();
        assert!(status.enabled);
        assert_eq!(status.phase, "degraded");
        assert_eq!(status.last_error, Some("archive_root_missing"));
        assert!(status.last_poll.is_none());
        assert_eq!(
            status.visibility,
            "dashboard_and_memory_keyword_fallback; live_meili_is_independent"
        );
    }

    fs::write(&fixture.source, b"not a valid zstd frame").unwrap();
    let mut index = fixture.open();
    for _ in 0..64 {
        poll_and_publish(&mut index, &lane, &metrics).unwrap();
        if lane.archive_status.lock().unwrap().last_error == Some("archive_source_errors") {
            let status = lane.archive_status.lock().unwrap();
            assert_eq!(status.phase, "degraded");
            assert_eq!(status.snapshot_hits, 0);
            assert!(!status.last_poll.as_ref().unwrap().errors.is_empty());
            return;
        }
    }
    panic!("corrupt archive was never reported as degraded");
}
