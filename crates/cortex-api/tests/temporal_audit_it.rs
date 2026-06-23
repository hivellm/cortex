//! Phase18 §3.9 — temporal classification + branch resolution audit
//! integration test.
//!
//! The classifier wedge emits three event kinds onto the
//! `cortex_audit` tracing target:
//!
//! 1. `temporal_classification` — one event per fused hit, carrying
//!    `state`, `action`, `doc_id`, `query_id`.
//! 2. `temporal_classification_summary` — one event per request,
//!    carrying state-bucket counts + action counts.
//! 3. `branch_resolution` — one event per request, carrying
//!    `branch` + `ancestry_chain` (default `<project>:main` until
//!    §4 P3 wires the request-side branch field).
//!
//! This IT installs a recording `tracing::Subscriber` for the
//! duration of one `Orchestrator::run` call and confirms every
//! envelope fires with the expected shape.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane, Overlay, VectorLane,
    VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, Props, QueryRequest, Scope};
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};

#[derive(Default)]
struct EmptyVectorLane;

#[async_trait]
impl VectorLane for EmptyVectorLane {
    async fn search(&self, _req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(Vec::new())
    }
}

struct StampedKeywordLane(Vec<LaneHit>);

#[async_trait]
impl KeywordLane for StampedKeywordLane {
    async fn search(&self, _req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(self.0.clone())
    }
}

fn now_unix() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

fn hit(doc_id: &str, score: f64, extras: &[(&str, serde_json::Value)]) -> LaneHit {
    let mut props = Props::new();
    for (k, v) in extras {
        props.insert((*k).to_string(), v.clone());
    }
    LaneHit {
        doc_id: doc_id.to_string(),
        text: format!("body-{doc_id}"),
        repo: Some("cortex".into()),
        path: Some(format!("docs/{doc_id}.md")),
        symbol: Some(format!("sym_{doc_id}")),
        content_hash: None,
        score,
        ts: 0,
        severity: None,
        extras: props,
        overlay: Overlay {
            source: cortex_api::lanes::LaneSource::Keyword,
            ..Overlay::default()
        },
    }
}

fn req() -> QueryRequest {
    QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope {
            repo: Some("cortex".into()),
            ..Scope::default()
        },
        query: "anything".into(),
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
    }
}

#[derive(Debug, Clone, Default)]
struct RecordedEvent {
    kind: Option<String>,
    fields: Vec<(String, String)>,
}

impl Visit for RecordedEvent {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "kind" {
            self.kind = Some(value.to_string());
        }
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .push((field.name().to_string(), format!("{value:?}")));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
}

#[derive(Default, Clone)]
struct RecordingSubscriber {
    events: Arc<Mutex<Vec<RecordedEvent>>>,
}

impl Subscriber for RecordingSubscriber {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "cortex_audit"
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut rec = RecordedEvent::default();
        event.record(&mut rec);
        if let Ok(mut g) = self.events.lock() {
            g.push(rec);
        }
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

#[test]
fn classifier_emits_per_hit_summary_and_branch_resolution_envelopes() {
    let now = now_unix();
    let past = now - 86_400;
    let soon = now + 3_600;

    let hits = vec![
        hit("valid", 1.0, &[]),
        hit("superseded", 1.5, &[("superseded_at_unix", json!(past))]),
        hit("boost", 0.9, &[("valid_to_unix", json!(soon))]),
    ];

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StampedKeywordLane(hits)),
        Arc::new(MemoryGraphLane::default()),
    );

    let recorder = RecordingSubscriber::default();
    let events_handle = recorder.events.clone();
    with_default(recorder, || {
        // The wedge runs inline on the tokio current-thread runtime
        // so the subscriber captures every event from this call.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (_resp, _) = orch.run(&req()).await;
        });
    });

    let events = events_handle.lock().unwrap().clone();

    let kinds: Vec<String> = events.iter().filter_map(|e| e.kind.clone()).collect();

    let per_hit = kinds
        .iter()
        .filter(|k| k.as_str() == "temporal_classification")
        .count();
    assert_eq!(
        per_hit, 3,
        "one temporal_classification per fused hit (3 hits in), got {kinds:?}"
    );

    let summary = kinds
        .iter()
        .filter(|k| k.as_str() == "temporal_classification_summary")
        .count();
    assert_eq!(summary, 1, "exactly one summary envelope per request");

    let branch = kinds
        .iter()
        .filter(|k| k.as_str() == "branch_resolution")
        .count();
    assert_eq!(branch, 1, "exactly one branch_resolution per request");

    // Per-hit envelope MUST carry `state` + `action` + `doc_id`.
    let any_hit_env = events
        .iter()
        .find(|e| e.kind.as_deref() == Some("temporal_classification"))
        .expect("no temporal_classification recorded");
    let names: Vec<&str> = any_hit_env.fields.iter().map(|(k, _)| k.as_str()).collect();
    assert!(names.contains(&"state"));
    assert!(names.contains(&"action"));
    assert!(names.contains(&"doc_id"));
    assert!(names.contains(&"query_id"));
}
