use cortex_local::*;
use serde_json::json;

fn query() -> AuditMetricsQuery {
    AuditMetricsQuery {
        repository: "Cortex".into(),
        after: None,
        before: None,
        include_archived: false,
        limit: 10_000,
    }
}
fn fixture(store: &LocalStore) -> AuditRecord {
    let memory = store
        .capture(CaptureRequest {
            id: Some("memory".into()),
            repository: "Cortex".into(),
            body: "Retain evidence".into(),
            ..Default::default()
        })
        .unwrap()
        .record;
    let attribution = AuditAttribution {
        client: Some("codex".into()),
        session_id: Some("session".into()),
        turn_id: Some("turn".into()),
        ..Default::default()
    };
    let reference = AuditReference::from_record(&memory, false).unwrap();
    for (id, outcome, latency_us) in [
        ("prepared", PreparationOutcome::Prepared, 100),
        ("other", PreparationOutcome::Prepared, 300),
        ("empty", PreparationOutcome::Empty, 0),
        ("error", PreparationOutcome::Error, 20),
    ] {
        let prepared = outcome == PreparationOutcome::Prepared;
        store
            .append_audit(AuditRequest {
                id: Some(id.into()),
                repository: "Cortex".into(),
                payload: AuditPayload::Delivery(DeliveryDraft {
                    query_id: format!("query-{id}"),
                    origin: AuditOrigin::Mcp,
                    attribution: attribution.clone(),
                    query_sha256: "a".repeat(64),
                    context_sha256: "b".repeat(64),
                    context_bytes: usize::from(prepared),
                    selected_count: usize::from(prepared),
                    omitted_count: 0,
                    references: if prepared {
                        vec![reference.clone()]
                    } else {
                        Vec::new()
                    },
                    outcome,
                    latency_us,
                    error: (outcome == PreparationOutcome::Error)
                        .then_some(AuditErrorCode::PreparationFailed),
                }),
            })
            .unwrap();
    }
    let event = store.capture(CaptureRequest { id: Some("stop".into()), repository: "Cortex".into(),
        kind: RecordKind::Event, body: format!("[memory@1 {}]", memory.content_sha256),
        source: Source { kind: "lifecycle_hook".into(), ..Default::default() },
        metadata: json!({"client":"codex","session_id":"session","turn_id":"turn","event":"Stop"}),
        ..Default::default() }).unwrap().record;
    let event_ref = AuditReference::from_record(&event, false).unwrap();
    let mut first_citation = None;
    for (id, method) in [
        ("related", LinkageMethod::NativeTurn),
        ("citation-one", LinkageMethod::ExactCitation),
        ("citation-two", LinkageMethod::ExactCitation),
    ] {
        let record = store
            .append_audit(AuditRequest {
                id: Some(id.into()),
                repository: "Cortex".into(),
                payload: AuditPayload::Observation(ObservationDraft {
                    delivery_id: Some("prepared".into()),
                    event: Some(event_ref.clone()),
                    attribution: attribution.clone(),
                    method,
                    evidence: if method == LinkageMethod::ExactCitation {
                        vec![reference.clone()]
                    } else {
                        Vec::new()
                    },
                    uncertainty: None,
                }),
            })
            .unwrap()
            .record;
        if id == "citation-one" {
            first_citation = Some(record);
        }
    }
    for (id, method, uncertainty) in [
        (
            "reported",
            LinkageMethod::ReportedUse,
            UncertaintyCode::ReportedOnly,
        ),
        (
            "missing-attribution",
            LinkageMethod::Unlinked,
            UncertaintyCode::MissingAttribution,
        ),
        (
            "missing-delivery",
            LinkageMethod::Unlinked,
            UncertaintyCode::MissingDelivery,
        ),
        (
            "ambiguous",
            LinkageMethod::Unlinked,
            UncertaintyCode::AmbiguousDelivery,
        ),
    ] {
        store
            .append_audit(AuditRequest {
                id: Some(id.into()),
                repository: "Cortex".into(),
                payload: AuditPayload::Observation(ObservationDraft {
                    delivery_id: None,
                    event: None,
                    attribution: AuditAttribution::default(),
                    method,
                    evidence: Vec::new(),
                    uncertainty: Some(uncertainty),
                }),
            })
            .unwrap();
    }
    store
        .append_audit(AuditRequest {
            id: Some("foreign".into()),
            repository: "cortex".into(),
            payload: AuditPayload::Observation(ObservationDraft {
                delivery_id: None,
                event: None,
                attribution: AuditAttribution::default(),
                method: LinkageMethod::Unlinked,
                evidence: Vec::new(),
                uncertainty: Some(UncertaintyCode::MissingAttribution),
            }),
        })
        .unwrap();
    first_citation.unwrap()
}

#[test]
fn metrics_keep_exclusive_evidence_classes_errors_and_delivery_denominators() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    fixture(&store);
    let metrics = store.audit_metrics(query()).unwrap();
    assert_eq!(metrics.matching_records, 11);
    assert_eq!(metrics.sampled_records, 11);
    assert_eq!(metrics.omitted_records, 0);
    assert!(!metrics.truncated);
    let d = &metrics.deliveries;
    assert_eq!(
        (d.attempts, d.prepared, d.empty, d.errors, d.unknown),
        (4, 2, 1, 1, 2)
    );
    assert_eq!(
        (
            d.latency.samples,
            d.latency.min_us,
            d.latency.max_us,
            d.latency.sum_us,
            d.latency.mean_us
        ),
        (4, Some(0), Some(300), 420, Some(105.0))
    );
    let o = &metrics.observations;
    assert_eq!(
        (o.total, o.related, o.captured_citations, o.unknown),
        (7, 1, 2, 4)
    );
    assert_eq!(
        (
            o.reported_use,
            o.missing_attribution,
            o.missing_delivery,
            o.ambiguous_delivery
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(metrics.distinct_cited_deliveries, 1);
    assert_eq!(metrics.prepared_deliveries_with_citation, 1);
    assert_eq!(metrics.citation_delivery_rate, Some(0.5));
    assert_eq!(
        metrics.citation_observations_with_delivery_outside_sample,
        0
    );
    let source_count = store.status(Some("Cortex")).unwrap().records;
    store.reindex().unwrap();
    assert_eq!(
        serde_json::to_value(store.audit_metrics(query()).unwrap()).unwrap(),
        serde_json::to_value(metrics).unwrap()
    );
    assert_eq!(store.status(Some("Cortex")).unwrap().records, source_count);
}

#[test]
fn metrics_windows_and_truncation_do_not_invent_receipt_denominators() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let citation = fixture(&store);
    let mut q = query();
    q.after = Some(citation.recorded_at.clone());
    let window = store.audit_metrics(q).unwrap();
    assert_eq!(window.matching_records, 6);
    assert_eq!(window.deliveries.attempts, 0);
    assert_eq!(window.observations.captured_citations, 2);
    assert_eq!(window.distinct_cited_deliveries, 1);
    assert_eq!(window.prepared_deliveries_with_citation, 0);
    assert_eq!(window.citation_delivery_rate, None);
    assert_eq!(window.citation_observations_with_delivery_outside_sample, 2);
    let mut q = query();
    q.before = Some(citation.recorded_at);
    let earlier = store.audit_metrics(q).unwrap();
    assert_eq!(earlier.observations.captured_citations, 0);
    assert_eq!(earlier.citation_delivery_rate, Some(0.0));
    let mut q = query();
    q.limit = 3;
    let bounded = store.audit_metrics(q).unwrap();
    assert_eq!(
        (
            bounded.matching_records,
            bounded.sampled_records,
            bounded.omitted_records
        ),
        (11, 3, 8)
    );
    assert!(bounded.truncated);
    assert_eq!(bounded.observations.total, 3);
    assert_eq!(bounded.citation_delivery_rate, None);
}

#[test]
fn archived_metrics_remain_explicit_and_empty_population_has_no_rate() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let empty = store.audit_metrics(query()).unwrap();
    assert_eq!(empty.sampled_records, 0);
    assert_eq!(empty.citation_delivery_rate, None);
    assert_eq!(empty.deliveries.latency.mean_us, None);
    fixture(&store);
    store
        .retain_audit(AuditRetentionRequest {
            repository: "Cortex".into(),
            cutoff: "9999-01-01T00:00:00Z".into(),
            limit: 100,
            apply: true,
        })
        .unwrap();
    let active = store.audit_metrics(query()).unwrap();
    assert_eq!(
        (
            active.matching_records,
            active.archive_controls,
            active.observations.total
        ),
        (5, 1, 4)
    );
    assert_eq!(active.deliveries.attempts, 0);
    assert_eq!(active.citation_delivery_rate, None);
    let mut q = query();
    q.include_archived = true;
    let all = store.audit_metrics(q).unwrap();
    assert_eq!(all.matching_records, 12);
    assert_eq!(all.citation_delivery_rate, Some(0.5));
    for limit in [0, 10_001] {
        let mut q = query();
        q.limit = limit;
        assert!(store.audit_metrics(q).is_err());
    }
    for value in ["garbage", "2026-01-01T12:00:00-03:00"] {
        let mut q = query();
        q.after = Some(value.into());
        assert!(store.audit_metrics(q).is_err());
    }
    let mut q = query();
    q.after = Some("2026-01-02T00:00:00Z".into());
    q.before = Some("2026-01-01T00:00:00Z".into());
    assert!(store.audit_metrics(q).is_err());
}
