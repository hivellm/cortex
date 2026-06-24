//! Phase21 §8.1 — access_decision audit envelope integration test.
//!
//! The `apply_acl_wedge` function emits two event kinds onto the
//! `cortex_audit` tracing target:
//!
//! 1. `access_decision` — one event per fused hit, carrying
//!    `principal_id`, `doc_id`, `fact_level`, `fact_compartments`,
//!    `clearance_level`, `verdict`, `reason`, `query_id`.
//! 2. `access_decision_summary` — one event per request, carrying
//!    `principal_id`, `evaluated`, `granted`, `denied`, `query_id`.
//!
//! This IT installs a recording `tracing::Subscriber` (mirroring
//! `temporal_audit_it.rs`) and confirms every envelope fires with the
//! correct shape and values when `run_with_principal` is called.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_api::lanes::{
    KeywordLane, KeywordRequest, LaneError, LaneHit, MemoryGraphLane, Overlay, VectorLane,
    VectorRequest,
};
use cortex_api::orchestrator::Orchestrator;
use cortex_api::types::{IncludeField, Intent, Props, QueryRequest, Scope};
use cortex_api::Principal;
use cortex_config::AccessControlConfig;
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};

// ---- Test doubles -------------------------------------------------------

#[derive(Default)]
struct EmptyVectorLane;

#[async_trait]
impl VectorLane for EmptyVectorLane {
    async fn search(&self, _req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(Vec::new())
    }
}

struct StaticKeywordLane(Vec<LaneHit>);

#[async_trait]
impl KeywordLane for StaticKeywordLane {
    async fn search(&self, _req: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok(self.0.clone())
    }
}

fn hit_classified(id: &str, level: u8, compartments: &[&str]) -> LaneHit {
    let mut extras = Props::new();
    extras.insert("class_level".to_string(), json!(level));
    if !compartments.is_empty() {
        let comps: Vec<serde_json::Value> = compartments.iter().map(|c| json!(c)).collect();
        extras.insert("class_compartments".to_string(), json!(comps));
    }
    LaneHit {
        doc_id: id.to_string(),
        text: id.to_string(),
        repo: Some("repo".into()),
        path: Some(format!("{id}.rs")),
        symbol: None,
        content_hash: None,
        score: 1.0,
        ts: 0,
        severity: None,
        extras,
        overlay: Overlay {
            source: cortex_api::lanes::LaneSource::Keyword,
            ..Default::default()
        },
    }
}

fn base_request() -> QueryRequest {
    QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope::default(),
        query: "test".into(),
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
    }
}

// ---- Recording subscriber -----------------------------------------------

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

fn field_value<'e>(ev: &'e RecordedEvent, name: &str) -> Option<&'e str> {
    ev.fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

// ---- Tests --------------------------------------------------------------

/// Phase21 §8.1 — one `access_decision` per hit + one `access_decision_summary`
/// per request, all carrying `principal_id`, `verdict`, and `reason`.
#[test]
fn access_decision_envelopes_fire_with_required_fields() {
    let hits = vec![
        // level=0 — passes clearance=1
        hit_classified("public-doc", 0, &[]),
        // level=2 — denied: above clearance=1
        hit_classified("confidential-doc", 2, &[]),
        // level=1, compartment=financial — denied: missing compartment
        hit_classified("financial-doc", 1, &["financial"]),
    ];

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StaticKeywordLane(hits)),
        Arc::new(MemoryGraphLane::new()),
    )
    .with_access_control(AccessControlConfig {
        enabled: true,
        ..AccessControlConfig::default()
    });

    let principal = Principal {
        id: "audit-tester".into(),
        clearance_level: 1,
        compartment_grants: vec![],
        roles: vec![],
    };

    let recorder = RecordingSubscriber::default();
    let events_handle = recorder.events.clone();
    with_default(recorder, || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (_resp, _) = orch.run_with_principal(&base_request(), &principal).await;
        });
    });

    let events = events_handle.lock().unwrap().clone();
    let kinds: Vec<Option<&str>> = events.iter().map(|e| e.kind.as_deref()).collect();

    // 3 per-hit envelopes + 1 summary.
    let per_hit_count = kinds.iter().filter(|k| **k == Some("access_decision")).count();
    assert_eq!(
        per_hit_count, 3,
        "one access_decision per fused hit (3 hits in), got {kinds:?}"
    );
    let summary_count = kinds
        .iter()
        .filter(|k| **k == Some("access_decision_summary"))
        .count();
    assert_eq!(summary_count, 1, "exactly one summary per request");

    // Per-hit envelope MUST carry the full set of required fields.
    let any_hit_env = events
        .iter()
        .find(|e| e.kind.as_deref() == Some("access_decision"))
        .expect("no access_decision recorded");
    let names: Vec<&str> = any_hit_env.fields.iter().map(|(k, _)| k.as_str()).collect();
    for required in &[
        "principal_id",
        "doc_id",
        "fact_level",
        "fact_compartments",
        "clearance_level",
        "verdict",
        "reason",
        "query_id",
    ] {
        assert!(
            names.contains(required),
            "field {required:?} missing from access_decision envelope; got {names:?}"
        );
    }

    // Summary MUST carry principal_id + evaluated/granted/denied counts.
    let summary = events
        .iter()
        .find(|e| e.kind.as_deref() == Some("access_decision_summary"))
        .expect("no access_decision_summary recorded");
    let snames: Vec<&str> = summary.fields.iter().map(|(k, _)| k.as_str()).collect();
    for required in &["principal_id", "evaluated", "granted", "denied", "query_id"] {
        assert!(
            snames.contains(required),
            "field {required:?} missing from access_decision_summary; got {snames:?}"
        );
    }
}

/// Phase21 §8.1 — reason field MUST distinguish the denial cause:
/// `above_clearance_level` vs `missing_compartment`.
#[test]
fn access_decision_reason_distinguishes_level_vs_compartment_denial() {
    let hits = vec![
        hit_classified("above-level", 3, &[]),
        hit_classified("missing-compart", 0, &["legal"]),
        hit_classified("granted-doc", 0, &[]),
    ];

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StaticKeywordLane(hits)),
        Arc::new(MemoryGraphLane::new()),
    )
    .with_access_control(AccessControlConfig {
        enabled: true,
        ..AccessControlConfig::default()
    });

    let principal = Principal {
        id: "reason-tester".into(),
        clearance_level: 1,
        compartment_grants: vec![],
        roles: vec![],
    };

    let recorder = RecordingSubscriber::default();
    let events_handle = recorder.events.clone();
    with_default(recorder, || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (_resp, _) = orch.run_with_principal(&base_request(), &principal).await;
        });
    });

    let events = events_handle.lock().unwrap().clone();

    let find_reason = |doc: &str| {
        events
            .iter()
            .find(|e| {
                e.kind.as_deref() == Some("access_decision")
                    && field_value(e, "doc_id").unwrap_or("") == doc
            })
            .and_then(|e| field_value(e, "reason"))
            .map(String::from)
    };

    assert_eq!(
        find_reason("above-level").as_deref(),
        Some("above_clearance_level"),
        "level=3 doc denied by clearance must reason=above_clearance_level"
    );
    assert_eq!(
        find_reason("missing-compart").as_deref(),
        Some("missing_compartment"),
        "level=0 doc requiring legal compartment must reason=missing_compartment"
    );
    assert_eq!(
        find_reason("granted-doc").as_deref(),
        Some("granted"),
        "public doc with no compartments must reason=granted"
    );
}

/// Phase21 §8.1 — principal_id field MUST carry the resolved principal
/// identity, not "anonymous", for authenticated callers.
#[test]
fn access_decision_carries_principal_id() {
    let hits = vec![hit_classified("doc-a", 0, &[])];

    let orch = Orchestrator::new(
        Arc::new(EmptyVectorLane),
        Arc::new(StaticKeywordLane(hits)),
        Arc::new(MemoryGraphLane::new()),
    )
    .with_access_control(AccessControlConfig {
        enabled: true,
        ..AccessControlConfig::default()
    });

    let principal = Principal {
        id: "alice".into(),
        clearance_level: 3,
        compartment_grants: vec!["*".into()],
        roles: vec![],
    };

    let recorder = RecordingSubscriber::default();
    let events_handle = recorder.events.clone();
    with_default(recorder, || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (_resp, _) = orch.run_with_principal(&base_request(), &principal).await;
        });
    });

    let events = events_handle.lock().unwrap().clone();

    let per_hit = events
        .iter()
        .find(|e| e.kind.as_deref() == Some("access_decision"))
        .expect("no access_decision event");
    assert_eq!(
        field_value(per_hit, "principal_id"),
        Some("alice"),
        "principal_id must be the resolved principal ID, not anonymous"
    );
    let summary = events
        .iter()
        .find(|e| e.kind.as_deref() == Some("access_decision_summary"))
        .expect("no access_decision_summary");
    assert_eq!(
        field_value(summary, "principal_id"),
        Some("alice"),
        "summary principal_id must also carry the resolved ID"
    );
}
