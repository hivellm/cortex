//! Cross-session continuity through the real pipeline and loopback query API.
//!
//! A synthetic prior-session turn and its pre-existing consolidation are
//! archived, then projected by the production archive loader. A fresh session
//! must receive that exact consolidation through real HTTP, query assembly,
//! and bundle formatting within the production 600 ms budget. Only retrieval
//! backends are in-memory; no running daemon, provider, or shared corpus is used.
//! This tests optional consolidation continuity, not LLM synthesis or the
//! separate durable daily-memory delivery protocol.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cortex_api::{
    build_router_with_auth_and_cfg, load_lane_hits, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, Orchestrator, QueryRequest, QueryResponse, QueryService,
};
use cortex_core::events::{
    ConsolidationDepth, ConsolidationGrain, ConsolidationPayload, ConsolidationScope, Envelope,
    TimeSpan,
};
use cortex_pre_thinking::metrics::Metrics;
use cortex_pre_thinking::pipeline::{run, ClosureQueryFn, PreThinkingBudget, PreThinkingInput};
use serde_json::{json, Value};

const REPO: &str = "continuity-fixture";
const PRIOR_SESSION: &str = "01K00000000000000000000001";
const FRESH_SESSION: &str = "01K00000000000000000000002";
const SOURCE_EVENT: &str = "01K00000000000000000000003";
const CONS_EVENT: &str = "01K00000000000000000000004";
const CONS_ID: &str = "cons-ses-2a417d3c7c87fe945e513b57";
const TITLE: &str = "Use compare-and-swap for concurrent correction writes";

fn envelope(event_id: &str, kind: &str, payload: Value) -> Envelope {
    serde_json::from_value(json!({
        "event_id": event_id,
        "schema_version": "1",
        "occurred_at": "2026-10-15T12:00:00.000Z",
        "session_id": PRIOR_SESSION,
        "stream": "live",
        "tool": "claude-code",
        "kind": kind,
        "context": {"repo": REPO, "platform": "win32"},
        "content_hash": cortex_core::content_hash(&payload).unwrap().to_string(),
        "payload": payload
    }))
    .expect("synthetic prior-session envelope must deserialize")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prior_session_consolidation_surfaces_in_fresh_session_bundle() {
    let fixture = tempfile::tempdir().unwrap();
    let repo = fixture.path().join("checkout");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(
        repo.join("cortex.toml"),
        format!("[cortex]\nid = \"{REPO}\"\n"),
    )
    .unwrap();

    let source = envelope(
        SOURCE_EVENT,
        "turn",
        json!({
            "user_message": "How should concurrent correction writes be guarded?",
            "assistant_message": TITLE
        }),
    );
    let source_ts = chrono::DateTime::parse_from_rfc3339(&source.occurred_at)
        .unwrap()
        .timestamp_millis();
    let payload = ConsolidationPayload {
        consolidation_id: CONS_ID.into(),
        grain: ConsolidationGrain::Session,
        scope: ConsolidationScope::SessionId(source.session_id.clone()),
        title: TITLE.into(),
        summary_markdown: format!(
            "{TITLE}. The prior session concluded that concurrent corrections must compare \
             the stored version before committing. Reject a stale writer instead of silently \
             overwriting another correction, and preserve the original source event for review."
        ),
        takeaways: vec![TITLE.into()],
        source_event_ids: vec![source.event_id.clone()],
        source_event_count: 1,
        model: "synthetic-fixture".into(),
        depth: ConsolidationDepth::Shallow,
        outcome_distribution: BTreeMap::new(),
        temporal_span: TimeSpan {
            start_ms: source_ts,
            end_ms: source_ts,
            duration_ms: 0,
        },
        repos: vec![REPO.into()],
        tags: Vec::new(),
    };
    cortex_core::validate::validate_consolidation_payload(&payload).unwrap();
    assert_eq!(
        payload.source_event_ids.as_slice(),
        std::slice::from_ref(&source.event_id)
    );
    assert_eq!(
        payload.scope,
        ConsolidationScope::SessionId(PRIOR_SESSION.into())
    );
    assert_ne!(source.session_id, FRESH_SESSION);
    let consolidation = envelope(
        CONS_EVENT,
        "consolidation",
        serde_json::to_value(&payload).unwrap(),
    );

    // Persist both fixtures before starting the fresh session. Loading must
    // fail the test if either event disappears or cannot be projected.
    let archive = fixture.path().join("archive");
    std::fs::create_dir(&archive).unwrap();
    let file = std::fs::File::create(archive.join("raw-00000.parquet")).unwrap();
    let mut writer = zstd::stream::write::Encoder::new(file, 1).unwrap();
    for event in [&source, &consolidation] {
        serde_json::to_writer(&mut writer, event).unwrap();
        writer.write_all(b"\n").unwrap();
    }
    writer.finish().unwrap();
    let (report, hits) = load_lane_hits(&archive);
    assert_eq!(report.files_visited, 1);
    assert_eq!(report.envelopes_parsed, 2);
    assert_eq!(report.hits_seeded, 2);
    assert_eq!(report.lines_dropped, 0);
    assert!(report.corrupted_files.is_empty());
    let cons_hit = hits
        .iter()
        .find(|hit| hit.symbol.as_deref() == Some("consolidation"))
        .expect("archived prior-session consolidation must be indexed")
        .clone();
    assert_eq!(cons_hit.doc_id, format!("archive|{CONS_EVENT}"));
    assert_eq!(cons_hit.extras["session_id"], PRIOR_SESSION);
    assert_eq!(cons_hit.extras["consolidation_id"], CONS_ID);
    assert_eq!(cons_hit.extras["title"], TITLE);
    assert_eq!(cons_hit.extras["source_event_count"], 1);
    assert_eq!(cons_hit.extras["body_markdown"], payload.summary_markdown);
    // Archive projection carries `title`, whereas the Meili projection uses
    // `consolidation_title`. The query API's archive fallback title is the
    // complete projected text. Pin it to this source rather than substituting
    // Meili metadata or accepting any consolidation in the response.
    assert!(payload.summary_markdown.len() <= 320);
    let projected_title = format!("[{}] {}", payload.title, payload.summary_markdown);
    assert_eq!(cons_hit.text, projected_title);
    let turn_hit = hits
        .iter()
        .find(|hit| hit.symbol.as_deref() == Some("turn"))
        .expect("exact source turn must be indexed")
        .clone();
    assert_eq!(turn_hit.doc_id, format!("archive|{SOURCE_EVENT}"));
    assert_eq!(turn_hit.extras["session_id"], PRIOR_SESSION);

    let keyword = Arc::new(MemoryKeywordLane::new());
    keyword.seed(&format!("cortex-{REPO}-consolidations"), vec![cons_hit]);
    keyword.seed(&format!("cortex-{REPO}-turns"), vec![turn_hit]);
    let service = QueryService::with_memory_defaults(Orchestrator::new(
        Arc::new(MemoryVectorLane::new()),
        keyword,
        Arc::new(MemoryGraphLane::new()),
    ));
    // Explicit defaults keep this fixture independent of user configuration.
    let app = build_router_with_auth_and_cfg(
        Arc::new(service),
        None,
        None,
        Arc::new(cortex_config::Config::default()),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/query", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(600))
        .build()
        .unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let query_fn = Arc::new(ClosureQueryFn(move |req: QueryRequest| {
        let client = client.clone();
        let endpoint = endpoint.clone();
        let captured = captured.clone();
        async move {
            let response = client
                .post(endpoint)
                .json(&req)
                .send()
                .await
                .expect("hermetic query API must be reachable")
                .error_for_status()
                .expect("hermetic query API must accept fresh-session query")
                .json::<QueryResponse>()
                .await
                .expect("real API query response must deserialize");
            captured.lock().unwrap().push((req, response.clone()));
            Some(response)
        }
    }));
    let prompt = format!(
        "I'm about to change correction writes. What did previous sessions conclude about {TITLE}?"
    );
    let input = PreThinkingInput {
        session_id: FRESH_SESSION,
        turn_id: "turn-1",
        user_prompt: &prompt,
        cwd: &repo,
        recent_files: &[],
        budget: PreThinkingBudget {
            bundle_bytes: 32 * 1024,
            time_ms: 600,
        },
        principal: None,
    };
    let out = run(&input, query_fn, Arc::new(Metrics::new())).await;
    server.abort();
    let _ = server.await;

    assert!(
        !out.fail_open,
        "fresh session lost continuity: {}",
        out.bundle
    );
    assert!(
        out.latency_ms <= 600,
        "production deadline exceeded: {} ms",
        out.latency_ms
    );
    let observed = observed.lock().unwrap();
    assert_eq!(
        observed.len(),
        1,
        "fresh turn must query the real API exactly once"
    );
    let (request, response) = &observed[0];
    assert_eq!(request.scope.repo.as_deref(), Some(REPO));
    assert_eq!(request.query, prompt);
    assert!(request.budget_ms <= 600);
    assert_eq!(response.results.consolidations.len(), 1);
    let delivered = &response.results.consolidations[0];
    assert_eq!(delivered.consolidation_id, payload.consolidation_id);
    assert_eq!(delivered.title, projected_title);
    assert_eq!(delivered.grain, "session");
    assert_eq!(delivered.ts, source_ts);
    assert_eq!(out.query_id.as_deref(), Some(response.query_id.as_str()));
    assert!(
        out.bundle.contains("## Consolidated context (1)"),
        "{}",
        out.bundle
    );
    assert!(
        out.bundle.contains(&format!("session/{CONS_ID}")),
        "{}",
        out.bundle
    );
    assert!(out.bundle.contains(TITLE), "{}", out.bundle);
}
