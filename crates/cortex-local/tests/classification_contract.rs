use cortex_local::classification::*;
use cortex_local::{Authority, CaptureRequest, LocalStore, Source};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn preparation_redacts_before_truncation_and_preserves_source() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let body = "password=inline-secret\n{\"api_key\":\"json-secret\"}\nTOKEN=env-secret\npostgres://alice:db-secret@example.invalid/db\nBearer bearer-secret\n-----BEGIN PRIVATE KEY-----\nprivate-secret\n-----END PRIVATE KEY-----\nUsed sk-proj-1234567890abcdef_1234 and sk-svcacct-1234567890abcdef_5678.\nAuthorization: Basic YWxpY2U6c2VjcmV0\nFound github_pat_1234567890abcdef1234567890 in prose.";
    let record = store
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            title: "Credential handling lesson".into(),
            body: body.into(),
            authority: Authority::Accepted,
            source: Source {
                locator: "private-path".into(),
                payload: json!({"do_not_send":"raw-original-secret"}),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record;
    let prepared = prepare(&record, DEFAULT_MODEL).unwrap();
    let outbound = serde_json::to_string(&prepared.request_body()).unwrap();
    for secret in [
        "inline-secret",
        "json-secret",
        "env-secret",
        "db-secret",
        "bearer-secret",
        "private-secret",
        "raw-original-secret",
        "private-path",
        "sk-proj-1234567890abcdef_1234",
        "sk-svcacct-1234567890abcdef_5678",
        "YWxpY2U6c2VjcmV0",
        "github_pat_1234567890abcdef1234567890",
    ] {
        assert!(!outbound.contains(secret), "leaked {secret}");
    }
    assert_eq!(prepared.state["input_truncated"], false);
    assert_eq!(store.get("Cortex", &record.id).unwrap().unwrap(), record);
    assert!(outbound.contains("REDACTED"));
}

#[test]
fn serialized_bounds_handle_unicode_escaping_and_state_shape() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let record = store
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            title: "\u{00e9}".repeat(4000),
            body: "\u{0001}\n\u{00e9}".repeat(10_000),
            ..Default::default()
        })
        .unwrap()
        .record;
    let prepared = prepare(&record, DEFAULT_MODEL).unwrap();
    assert!(serde_json::to_vec(&prepared.request_body()).unwrap().len() <= MAX_REQUEST_BYTES);
    assert_eq!(prepared.state["input_truncated"], true);
    assert!(prepared.validate().is_ok());
    let mut altered = prepared.clone();
    altered.state["candidate"]["body"] = json!("changed");
    assert!(altered.validate().is_err());
    altered.state["credentials"] = json!("not allowed");
    assert!(altered.validate().is_err());
}

#[test]
fn advisory_validation_distinguishes_confidence_from_probability() {
    let mut advice = ClassificationAdvice {
        category: Category::Lesson,
        probabilities: BTreeMap::from([
            (Category::Decision, 0.12),
            (Category::Lesson, 0.88),
            (Category::Preference, 0.0),
            (Category::Fact, 0.0),
            (Category::Transient, 0.0),
            (Category::Unknown, 0.0),
        ]),
        confidence: 0.81,
        durable_probability: 0.95,
        model: DEFAULT_MODEL.into(),
        usage: TokenUsage {
            input_tokens: 124,
            output_tokens: 32,
        },
    };
    assert!(advice.validate(DEFAULT_MODEL).is_ok());
    advice.category = Category::Decision;
    assert!(advice.validate(DEFAULT_MODEL).is_err());
    advice.category = Category::Lesson;
    advice.confidence = f64::NAN;
    assert!(advice.validate(DEFAULT_MODEL).is_err());
    advice.confidence = 0.81;
    advice.probabilities.remove(&Category::Unknown);
    assert!(advice.validate(DEFAULT_MODEL).is_err());
}

#[test]
fn classification_identity_includes_exact_scope_and_revision() {
    let root = tempfile::tempdir().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let first = store
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            id: Some("same".into()),
            body: "stable fact".into(),
            ..Default::default()
        })
        .unwrap()
        .record;
    let second = store
        .capture(CaptureRequest {
            repository: "cortex".into(),
            id: Some("same".into()),
            body: "stable fact".into(),
            ..Default::default()
        })
        .unwrap()
        .record;
    let first_key = prepare(&first, DEFAULT_MODEL).unwrap().cache_key().unwrap();
    assert_ne!(
        first_key,
        prepare(&second, DEFAULT_MODEL)
            .unwrap()
            .cache_key()
            .unwrap()
    );
    assert_ne!(
        first_key,
        prepare(&first, "jev-preview").unwrap().cache_key().unwrap()
    );
}
