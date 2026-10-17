use super::*;
use cortex_storage::daily_memory::{MemoryCategory, MemoryOrigin};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn candidate(text: &str) -> MemoryRecord {
    MemoryRecord {
        id: "local-id".into(),
        scope: MemoryScope::Repository("Exact/Repo".into()),
        title: "Explicit memory".into(),
        body: text.into(),
        category: MemoryCategory::Decision,
        capture_kind: None,
        status: MemoryStatus::Accepted,
        origin: MemoryOrigin::default(),
        sources: vec![],
        branch: None,
        path: None,
        pinned: false,
        stale: false,
        version: 1,
        event_id: "private-event-id".into(),
        captured_at: "2026-10-16T00:00:00Z".into(),
        content_hash: "local-hash".into(),
        supersedes: None,
    }
}
fn response() -> Value {
    json!({"model":MODEL,"answers":{
        "durable":{"type":"noul","noul":0.93},
        "kind":{"type":"choice","choice":"decision","probabilities":{"decision":0.8,"lesson":0.15,"transient":0.05},"confidence":0.8},
        "scope":{"type":"choice","choice":"repo","probabilities":{"repo":0.9,"global":0.03,"unclear":0.07},"confidence":0.86}
    },"usage":{"input_tokens":931,"output_tokens":74}})
}
fn provider() -> Provider {
    Provider::new("synthetic-configured-key-01934".into()).unwrap()
}

#[tokio::test]
async fn wire_state_redacts_all_fields_without_changing_explicit_memory() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response()))
        .expect(1)
        .mount(&server)
        .await;
    let provider = Provider::for_mock(
        "synthetic-configured-key-01934".into(),
        format!("{}/v1/systemone", server.uri()),
    );
    let mut record = candidate(
        r#"{"nested":{"password":"private-password","api_key":"private-api-key"},"text":"synthetic-configured-key-01934","token":"ts_deadbeef123"}"#,
    );
    record.title = "Bearer abcd+/~== sk-proj-privatefixture sk-svcacct-privatefixture".into();
    record.origin.session_id = Some("private-session".into());
    let mut neighbor = candidate("-----BEGIN PRIVATE KEY-----\nprivate-unfinished-pem");
    neighbor.id = "neighbor-private-id".into();
    neighbor.title = "ts_neighborcredential Authorization: Basic dXNlcjpwYXNz".into();
    let original = record.body.clone();
    let prepared = provider.prepare(&record, 2, &[neighbor]).unwrap();
    // The transport still sends one request; its strict answer-coverage failure is harmless.
    let evaluated = provider.evaluate(&prepared).await;
    assert!(evaluated.result.is_err());
    let requests = server.received_requests().await.unwrap();
    let wire = String::from_utf8(requests[0].body.clone()).unwrap();
    for secret in [
        "private-password",
        "private-api-key",
        "synthetic-configured-key-01934",
        "ts_deadbeef123",
        "abcd+/~==",
        "sk-proj-privatefixture",
        "sk-svcacct-privatefixture",
        "dXNlcjpwYXNz",
        "private-unfinished-pem",
        "ts_neighborcredential",
        "private-session",
        "private-event-id",
        "neighbor-private-id",
        "local-hash",
        "Exact/Repo",
    ] {
        assert!(
            !wire.contains(secret),
            "egress leaked synthetic fixture {secret}"
        );
    }
    assert_eq!(record.body, original);
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer synthetic-configured-key-01934"
    );
    assert_eq!(prepared.request["questions"].as_object().unwrap().len(), 4);
    assert_eq!(prepared.request["model"], MODEL);
}

#[test]
fn nested_secret_fields_and_escaped_literals_are_sanitized() {
    let mut state = json!({"safe":[{"API_KEY":{"nested":"secret-object-value"},"text":"ts_abc123"}],
        "text":r#"{"password":"hidden","value":"\u0073ynthetic-configured-key-01934"}"#});
    sanitize(&mut state, "synthetic-configured-key-01934");
    let rendered = state.to_string();
    for secret in [
        "secret-object-value",
        "ts_abc123",
        "hidden",
        "configured-key-01934",
    ] {
        assert!(!rendered.contains(secret));
    }
    for armor in ["-----BEGIN PGP PRIVATE KEY BLOCK-----\nprivate-pgp-material\n-----END PGP PRIVATE KEY BLOCK-----",
        "-----BEGIN PGP PRIVATE KEY BLOCK-----\nprivate-pgp-material"] {
        let prepared = provider().prepare(&candidate(armor),1,&[]).unwrap();
        assert!(!prepared.request.to_string().contains("private-pgp-material"));
    }
}

#[test]
fn byte_limit_measures_complete_sanitized_state_without_prefix_truncation() {
    let provider = provider();
    let mut record = candidate("");
    let overhead = serde_json::to_vec(&provider.prepare(&record, 1, &[]).unwrap().request["state"])
        .unwrap()
        .len();
    record.body = "x".repeat(MAX_STATE - overhead);
    assert!(provider.prepare(&record, 1, &[]).is_ok());
    record.body.push('x');
    assert_eq!(
        provider.prepare(&record, 1, &[]).err(),
        Some(AdviceFailure::StateTooLarge)
    );
    record.body = "\u{e9}".repeat((MAX_STATE - overhead) / 2 + 1);
    assert_eq!(
        provider.prepare(&record, 1, &[]).err(),
        Some(AdviceFailure::StateTooLarge)
    );
}

#[test]
fn cache_identity_tracks_scope_version_declarations_and_neighbor_revision() {
    let provider = provider();
    let base = candidate("Use transactions for writes.");
    let key = provider.prepare(&base, 1, &[]).unwrap().cache_key;
    let mut variants = vec![];
    let mut other = base.clone();
    other.scope = MemoryScope::Repository("exact/repo".into());
    variants.push(other);
    let mut other = base.clone();
    other.version += 1;
    variants.push(other);
    let mut other = base.clone();
    other.category = MemoryCategory::Lesson;
    variants.push(other);
    let mut other = base.clone();
    other.body.push('!');
    variants.push(other);
    for other in variants {
        assert_ne!(key, provider.prepare(&other, 1, &[]).unwrap().cache_key);
    }
    assert_ne!(key, provider.prepare(&base, 2, &[]).unwrap().cache_key);
    assert_eq!(key, provider.prepare(&base, 1, &[]).unwrap().cache_key);
}

#[test]
fn ineligible_or_cross_scope_neighbors_never_leave_process() {
    let provider = provider();
    let record = candidate("Use transactions.");
    let mut other = record.clone();
    other.id = "other".into();
    for status in [
        MemoryStatus::Proposed,
        MemoryStatus::Superseded,
        MemoryStatus::Deprecated,
    ] {
        other.status = status;
        assert_eq!(
            provider.prepare(&record, 1, &[other.clone()]).err(),
            Some(AdviceFailure::UnsafeState)
        );
    }
    other.status = MemoryStatus::Accepted;
    other.scope = MemoryScope::Global;
    assert!(provider.prepare(&record, 1, &[other.clone()]).is_err());
    other.scope = record.scope.clone();
    other.stale = true;
    assert!(provider.prepare(&record, 1, &[other.clone()]).is_err());
    other.stale = false;
    assert!(provider
        .prepare(&record, 1, &[other.clone(), other])
        .is_err());
    assert!(provider
        .prepare(&record, 1, std::slice::from_ref(&record))
        .is_err());
}

#[test]
fn exact_typed_contract_rejects_bad_probabilities_options_types_and_coverage() {
    let prepared = provider()
        .prepare(&candidate("Use transactions."), 1, &[])
        .unwrap();
    let good = response();
    assert!(decode_response(&prepared, &good, true).is_ok());
    let mutations = [
        ("/model", json!("jev-latest")),
        ("/answers/durable/type", json!("score")),
        ("/answers/durable/noul", json!(-0.1)),
        ("/answers/durable/noul", json!(1.01)),
        ("/answers/durable/noul", json!("NaN")),
        ("/answers/kind/choice", json!("invented")),
        ("/answers/kind/confidence", json!(2)),
        ("/answers/kind/probabilities/decision", json!(0.7)),
        ("/answers/kind/choice", json!("lesson")),
        ("/answers/scope/type", json!("noul")),
    ];
    for (pointer, value) in mutations {
        let mut invalid = good.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            decode_response(&prepared, &invalid, true).is_err(),
            "accepted {pointer}"
        );
    }
    let mut invalid = good.clone();
    invalid["answers"]["extra"] = json!({"type":"noul","noul":0.5});
    assert!(decode_response(&prepared, &invalid, true).is_err());
    let mut invalid = good.clone();
    invalid["answers"]["kind"]["probabilities"]["invented"] = json!(0);
    assert!(decode_response(&prepared, &invalid, true).is_err());
    assert!(decode_response(&prepared, &good, false).is_err());
}

#[test]
fn cache_validation_rejects_changed_neighbor_identity_and_unbounded_values() {
    let provider = provider();
    let record = candidate("Use transactions.");
    let mut neighbor = record.clone();
    neighbor.id = "neighbor".into();
    let prepared = provider.prepare(&record, 2, &[neighbor]).unwrap();
    let mut raw = response();
    raw["answers"]["conflict_0"] = json!({"type":"noul","noul":0.8});
    let advice = decode_response(&prepared, &raw, true).unwrap();
    assert!(prepared
        .decode_cached(&serde_json::to_string(&advice).unwrap())
        .is_ok());
    let mut wrong = advice.clone();
    wrong.conflicts[0].memory_id = "invented".into();
    assert!(prepared
        .decode_cached(&serde_json::to_string(&wrong).unwrap())
        .is_err());
    let mut wrong = advice.clone();
    wrong.kind.confidence = f64::NAN;
    assert!(prepared.validate(&wrong).is_err());
    let mut wrong = advice;
    wrong.conflicts[0].version += 1;
    assert!(prepared.validate(&wrong).is_err());
}

#[tokio::test]
async fn successful_response_records_real_usage_negative_advice_stays_advisory() {
    let server = MockServer::start().await;
    let mut raw = response();
    raw["answers"]["durable"]["noul"] = json!(0.01);
    raw["answers"]["kind"] = json!({"type":"choice","choice":"transient","probabilities":{"decision":0.01,"lesson":0.01,"transient":0.98},"confidence":0.95});
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(raw))
        .expect(1)
        .mount(&server)
        .await;
    let provider = Provider::for_mock("synthetic-key".into(), server.uri());
    let record = candidate("Status: build running.");
    let result = provider
        .evaluate(&provider.prepare(&record, 1, &[]).unwrap())
        .await;
    let advice = result.result.unwrap();
    assert_eq!(advice.kind.choice, "transient");
    assert_eq!(advice.durable, 0.01);
    assert_eq!(result.usage.unwrap().input_tokens, 931);
    assert_eq!(record.status, MemoryStatus::Accepted);
}

#[tokio::test]
async fn malformed_answers_preserve_known_usage_without_caching_success() {
    let server = MockServer::start().await;
    let mut raw = response();
    raw["answers"]["durable"]["noul"] = json!(4);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(raw))
        .mount(&server)
        .await;
    let provider = Provider::for_mock("synthetic-key".into(), server.uri());
    let result = provider
        .evaluate(
            &provider
                .prepare(&candidate("Use transactions."), 1, &[])
                .unwrap(),
        )
        .await;
    assert!(matches!(result.result, Err(AdviceFailure::InvalidResponse)));
    assert_eq!(result.usage.unwrap().input_tokens, 931);
}

#[tokio::test]
async fn response_limit_and_malformed_json_fail_closed() {
    for (body, expected) in [
        (
            "x".repeat(MAX_RESPONSE + 1),
            AdviceFailure::ResponseTooLarge,
        ),
        ("{".into(), AdviceFailure::InvalidResponse),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        let provider = Provider::for_mock("synthetic-key".into(), server.uri());
        let result = provider
            .evaluate(
                &provider
                    .prepare(&candidate("Use transactions."), 1, &[])
                    .unwrap(),
            )
            .await;
        assert!(matches!(result.result,Err(reason) if reason==expected));
        assert!(result.usage.is_none());
    }
}

#[tokio::test]
async fn no_redirect_no_retry_and_bounded_stalled_request() {
    let target = MockServer::start().await;
    for status in [302, 429, 529] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("Location", target.uri())
                    .insert_header("Retry-After", "1"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let provider = Provider::for_mock("synthetic-key".into(), server.uri());
        let result = provider
            .evaluate(
                &provider
                    .prepare(&candidate("Use transactions."), 1, &[])
                    .unwrap(),
            )
            .await;
        assert!(matches!(result.result, Err(AdviceFailure::Unavailable)));
    }
    assert!(target.received_requests().await.unwrap().is_empty());
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(response())
                .set_delay(Duration::from_secs(3)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = Provider::for_mock("synthetic-key".into(), server.uri());
    let start = std::time::Instant::now();
    let result = provider
        .evaluate(
            &provider
                .prepare(&candidate("Use transactions."), 1, &[])
                .unwrap(),
        )
        .await;
    assert!(matches!(result.result, Err(AdviceFailure::Timeout)));
    assert!(start.elapsed() < Duration::from_millis(2600));
    assert!(result.usage.is_none());
}

#[tokio::test]
async fn streamed_response_without_content_length_obeys_byte_cap() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 8192];
        let _ = stream.read(&mut request).await.unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n").await.unwrap();
        let body = "x".repeat(MAX_RESPONSE + 1);
        let _ = stream
            .write_all(format!("{:x}\r\n{}\r\n0\r\n\r\n", body.len(), body).as_bytes())
            .await;
    });
    let provider = Provider::for_mock("synthetic-key".into(), format!("http://{address}"));
    let prepared = provider
        .prepare(&candidate("Use transactions."), 1, &[])
        .unwrap();
    let result = provider.evaluate(&prepared).await;
    assert!(matches!(
        result.result,
        Err(AdviceFailure::ResponseTooLarge)
    ));
    server.await.unwrap();
}
