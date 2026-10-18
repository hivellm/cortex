//! Public synthetic classification scoring through the production loopback provider and queue.
use cortex_local::{
    classification::{
        queue::{ClassificationQueue, DrainOptions, JobStatus},
        Category, ClassificationAdvice, FailureKind, Provider, DEFAULT_MODEL,
    },
    Authority, CaptureRequest, LocalStore, Record, RecordKind, Relation, Source,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Fixture {
    schema_version: u32,
    license: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    repository: String,
    title: String,
    body: String,
    authority: Authority,
    expected_category: Category,
    synthetic_response: Value,
}

fn fixture() -> Fixture {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/classification-eval-v1/cases.json")).unwrap();
    assert_eq!(fixture.schema_version, 1);
    assert_eq!(fixture.license, "CC0-1.0");
    assert_eq!(fixture.cases.len(), 6);
    fixture
}

fn capture(store: &LocalStore, case: &Case) -> Record {
    store
        .capture(CaptureRequest {
            id: Some(case.id.clone()),
            repository: case.repository.clone(),
            kind: RecordKind::Note,
            authority: case.authority,
            title: case.title.clone(),
            body: case.body.clone(),
            source: Source {
                kind: "public_fictional_fixture".into(),
                locator: format!("fiction://classification-eval-v1/{}", case.id),
                source_id: Some(case.id.clone()),
                payload: json!({"license":"CC0-1.0","fictional":true}),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
        .record
}

fn options(repository: &str) -> DrainOptions {
    DrainOptions {
        repository: repository.into(),
        max_jobs: 1,
        deadline: Duration::from_secs(5),
    }
}

fn loopback(status: u16, body: Value, delay: Duration) -> (Provider, thread::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "fixture server accept deadline");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept failed: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        let header_end = loop {
            let mut buffer = [0; 1024];
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "fixture request truncated");
            bytes.extend_from_slice(&buffer[..count]);
            assert!(bytes.len() < 32768, "fixture request exceeded bound");
            if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        assert!(header_end + length <= 32768);
        while bytes.len() < header_end + length {
            let mut buffer = [0; 4096];
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "fixture request body truncated");
            bytes.extend_from_slice(&buffer[..count]);
            assert!(Instant::now() < deadline, "fixture read deadline");
        }
        let request: Value =
            serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
        thread::sleep(delay);
        let body = serde_json::to_vec(&body).unwrap();
        let response = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nRetry-After: 2\r\nConnection: close\r\n\r\n", body.len());
        // A timeout intentionally closes the client before this synthetic response.
        let _ = stream
            .write_all(response.as_bytes())
            .and_then(|()| stream.write_all(&body));
        request
    });
    let provider = Provider::for_loopback(
        Some("public-inert-fixture-token".into()),
        Duration::from_secs(1),
        &format!("http://{address}/v1/systemone"),
    )
    .unwrap();
    (provider, handle)
}

#[tokio::test]
async fn public_mocked_quality_and_probabilities_preserve_original_authority() {
    let fixture = fixture();
    let mut correct = 0;
    let mut unknown = 0;
    let mut cases = Vec::new();
    for case in &fixture.cases {
        let home = tempfile::tempdir().unwrap();
        let store = LocalStore::open(home.path()).unwrap();
        let original = capture(&store, case);
        let queue = ClassificationQueue::new(store.clone());
        let job = queue
            .enqueue(&case.repository, &case.id, DEFAULT_MODEL)
            .unwrap();
        let (provider, handle) = loopback(200, case.synthetic_response.clone(), Duration::ZERO);
        let report = queue
            .drain(&provider, options(&case.repository))
            .await
            .unwrap();
        let request = handle.join().unwrap();
        assert_eq!(request["model"], DEFAULT_MODEL);
        assert_eq!(
            request["questions"],
            serde_json::from_str::<Value>(include_str!(
                "fixtures/classification-eval-v1/questions.json"
            ))
            .unwrap()
        );
        assert_eq!(report.completed, 1);
        let result = store
            .get(&case.repository, &format!("classification:{}", job.key))
            .unwrap()
            .unwrap();
        let advice: ClassificationAdvice =
            serde_json::from_value(result.metadata["advice"].clone()).unwrap();
        advice.validate(DEFAULT_MODEL).unwrap();
        let matches = advice.category == case.expected_category;
        correct += usize::from(matches);
        unknown += usize::from(advice.category == Category::Unknown);
        assert!((advice.probabilities.values().sum::<f64>() - 1.0).abs() < 1e-10);
        assert_eq!(advice.confidence, 0.65);
        assert_eq!(advice.probabilities[&advice.category], 0.75);
        assert_eq!(result.kind, RecordKind::Event);
        assert_eq!(result.authority, Authority::Observation);
        assert_eq!(result.links.len(), 1);
        assert_eq!(result.links[0].relation, Relation::DerivedFrom);
        assert_eq!(result.links[0].target_id, original.id);
        assert_eq!(
            store.get(&case.repository, &case.id).unwrap().unwrap(),
            original
        );
        let finished = queue.get(&case.repository, &job.key).unwrap().unwrap();
        assert_eq!(finished.status, JobStatus::Completed);
        assert_eq!(finished.known_input_tokens, 13);
        assert_eq!(finished.known_output_tokens, 5);
        assert_eq!(finished.unknown_usage_attempts, 0);
        cases.push(json!({"id":case.id,"expected":case.expected_category,"synthetic_actual":advice.category,"correct":matches,"probabilities":advice.probabilities,"source_hash_preserved":true}));
    }
    assert_eq!(
        correct, 4,
        "two intentional synthetic mistakes must remain visible"
    );
    assert_eq!(unknown, 2);
    println!(
        "{}",
        json!({"schema_version":1,"mode":"offline_synthetic_loopback","cases":cases,"correct":correct,"total":6,"accuracy":correct as f64 / 6.0,"unknown_answers":unknown,"known_input_tokens":78,"known_output_tokens":30,"claim":"Synthetic contract scoring only; not observed Jev accuracy or billed usage"})
    );
}

#[tokio::test]
async fn malformed_distributions_schema_and_model_never_change_source() {
    let fixture = fixture();
    let case = &fixture.cases[0];
    for mutation in ["distribution", "schema", "model", "missing_category"] {
        let home = tempfile::tempdir().unwrap();
        let store = LocalStore::open(home.path()).unwrap();
        let original = capture(&store, case);
        let queue = ClassificationQueue::new(store.clone());
        let job = queue
            .enqueue(&case.repository, &case.id, DEFAULT_MODEL)
            .unwrap();
        let mut body = case.synthetic_response.clone();
        match mutation {
            "distribution" => body["answers"]["category"]["probabilities"]["decision"] = json!(0.3),
            "schema" => body["answers"]["category"]["type"] = json!("number"),
            "model" => body["model"] = json!("different-model"),
            "missing_category" => {
                body["answers"]["category"]["probabilities"]
                    .as_object_mut()
                    .unwrap()
                    .remove("unknown");
            }
            _ => unreachable!(),
        }
        let (provider, handle) = loopback(200, body, Duration::ZERO);
        let report = queue
            .drain(&provider, options(&case.repository))
            .await
            .unwrap();
        handle.join().unwrap();
        assert_eq!(report.failed, 1, "{mutation}");
        let failed = queue.get(&case.repository, &job.key).unwrap().unwrap();
        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(
            failed.last_failure.unwrap().kind,
            FailureKind::InvalidResponse
        );
        assert_eq!(
            store.get(&case.repository, &case.id).unwrap().unwrap(),
            original
        );
        assert!(store
            .get(&case.repository, &format!("classification:{}", job.key))
            .unwrap()
            .is_none());
    }
}

#[tokio::test]
async fn rate_limit_and_timeout_remain_retryable_with_unknown_usage() {
    let fixture = fixture();
    let case = &fixture.cases[0];
    for (status, delay, kind) in [
        (429, Duration::ZERO, FailureKind::RateLimited),
        (200, Duration::from_millis(1250), FailureKind::Timeout),
    ] {
        let home = tempfile::tempdir().unwrap();
        let store = LocalStore::open(home.path()).unwrap();
        let original = capture(&store, case);
        let queue = ClassificationQueue::new(store.clone());
        let job = queue
            .enqueue(&case.repository, &case.id, DEFAULT_MODEL)
            .unwrap();
        let (provider, handle) = loopback(status, case.synthetic_response.clone(), delay);
        let report = queue
            .drain(&provider, options(&case.repository))
            .await
            .unwrap();
        handle.join().unwrap();
        assert_eq!(report.pending, 1);
        let pending = queue.get(&case.repository, &job.key).unwrap().unwrap();
        assert_eq!(pending.status, JobStatus::Pending);
        assert_eq!(pending.attempts, 1);
        assert_eq!(pending.unknown_usage_attempts, 1);
        assert_eq!(pending.known_input_tokens, 0);
        assert_eq!(pending.known_output_tokens, 0);
        let failure = pending.last_failure.unwrap();
        assert_eq!(failure.kind, kind);
        assert!(failure.retryable);
        assert_eq!(
            store.get(&case.repository, &case.id).unwrap().unwrap(),
            original
        );
        assert!(store
            .get(&case.repository, &format!("classification:{}", job.key))
            .unwrap()
            .is_none());
    }
}
