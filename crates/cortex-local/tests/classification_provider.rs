use cortex_local::{
    classification::{
        prepare, Category, FailureKind, PreparedClassification, Provider, DEFAULT_MODEL,
        MAX_RESPONSE_BYTES,
    },
    CaptureRequest, LocalStore,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

const KEY: &str = "synthetic-provider-credential";

struct Request {
    headers: String,
    body: Value,
}

fn input() -> PreparedClassification {
    let home = tempfile::tempdir().unwrap();
    let record = LocalStore::open(home.path())
        .unwrap()
        .capture(CaptureRequest {
            repository: "private-scope-not-for-provider".into(),
            id: Some("private-record-id".into()),
            title: "storage choice".into(),
            body: "Keep canonical source records across rebuilds.".into(),
            ..Default::default()
        })
        .unwrap()
        .record;
    prepare(&record, DEFAULT_MODEL).unwrap()
}

fn rehash(input: &mut PreparedClassification) {
    input.request_sha256 = format!(
        "{:x}",
        Sha256::digest(cortex_core::canonical_json::canonicalize(&input.request_body()).unwrap())
    );
}

fn answer() -> Value {
    json!({"model":DEFAULT_MODEL,"usage":{"input_tokens":83,"output_tokens":21},"answers":{
        "category":{"type":"choice","choice":"decision","probabilities":{"decision":0.88,"lesson":0.04,"preference":0.02,"fact":0.02,"transient":0.02,"unknown":0.02},"confidence":0.81},
        "durable":{"type":"noul","noul":0.93}
    }})
}

fn read_request(stream: &mut TcpStream, buffer: &mut [u8], deadline: Instant) -> usize {
    loop {
        match stream.read(buffer) {
            Ok(count) => return count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("fixture request read: {error}"),
        }
    }
}

fn server(
    respond: impl FnOnce(&mut TcpStream) + Send + 'static,
) -> (String, thread::JoinHandle<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let started = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(5),
                        "provider never contacted fixture"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let end = loop {
            let mut buffer = [0; 1024];
            let count = read_request(&mut stream, &mut buffer, deadline);
            assert_ne!(count, 0, "request ended before headers");
            bytes.extend_from_slice(&buffer[..count]);
            assert!(bytes.len() < 65536);
            if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .expect("bounded request has content-length");
        while bytes.len() < end + length {
            let mut buffer = [0; 4096];
            let count = read_request(&mut stream, &mut buffer, deadline);
            assert_ne!(count, 0, "request body truncated");
            bytes.extend_from_slice(&buffer[..count]);
        }
        let body = serde_json::from_slice(&bytes[end..end + length]).unwrap();
        respond(&mut stream);
        Request { headers, body }
    });
    (format!("http://{address}/v1/systemone"), handle)
}

fn response(status: u16, extra: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn json_server(value: Value) -> (String, thread::JoinHandle<Request>) {
    let bytes = response(
        200,
        "Content-Type: application/json\r\n",
        &serde_json::to_vec(&value).unwrap(),
    );
    server(move |stream| stream.write_all(&bytes).unwrap())
}

#[tokio::test]
async fn exact_authenticated_contract_preserves_independent_confidence_and_usage() {
    let mut value = answer();
    value["unknown_provider_field"] = json!(KEY);
    value["usage"]["ignored"] = json!(KEY);
    let (endpoint, handle) = json_server(value);
    let provider =
        Provider::for_loopback(Some(KEY.into()), Duration::from_secs(2), &endpoint).unwrap();
    let prepared = input();
    let advice = provider.classify(&prepared).await.unwrap();
    assert_eq!(advice.category, Category::Decision);
    assert_eq!(advice.confidence, 0.81);
    assert_eq!(advice.probabilities[&Category::Decision], 0.88);
    assert_eq!(advice.usage.input_tokens, 83);
    assert_eq!(advice.usage.output_tokens, 21);
    assert_eq!(advice.durable_probability, 0.93);
    assert!(!serde_json::to_string(&advice).unwrap().contains(KEY));
    assert!(!format!("{provider:?}").contains(KEY));
    let request = handle.join().unwrap();
    assert!(request
        .headers
        .starts_with("POST /v1/systemone HTTP/1.1\r\n"));
    assert!(request
        .headers
        .to_ascii_lowercase()
        .contains(&format!("authorization: bearer {KEY}")));
    assert!(request
        .headers
        .to_ascii_lowercase()
        .contains("content-type: application/json"));
    assert_eq!(request.body, prepared.request_body());
    assert!(!request
        .body
        .to_string()
        .contains("private-scope-not-for-provider"));
    assert!(!request.body.to_string().contains("private-record-id"));
}

#[tokio::test]
async fn no_key_invalid_request_and_secret_echo_never_contact_http() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let missing = Provider::for_loopback(None, Duration::from_secs(1), &endpoint).unwrap();
    assert!(!missing.is_configured());
    assert_eq!(
        missing.classify(&input()).await.unwrap_err().kind,
        FailureKind::Unavailable
    );
    let provider =
        Provider::for_loopback(Some(KEY.into()), Duration::from_secs(1), &endpoint).unwrap();
    let mut bad = input();
    bad.request_sha256 = "0".repeat(64);
    assert_eq!(
        provider.classify(&bad).await.unwrap_err().kind,
        FailureKind::Rejected
    );
    let mut echo = input();
    echo.state["candidate"]["body"] = json!(KEY);
    rehash(&mut echo);
    let error = provider.classify(&echo).await.unwrap_err();
    assert_eq!(error.kind, FailureKind::Rejected);
    assert!(!error.to_string().contains(KEY));
    let escaped_key = "synthetic-credential-\"with\\escapes";
    let escaped_provider =
        Provider::for_loopback(Some(escaped_key.into()), Duration::from_secs(1), &endpoint)
            .unwrap();
    echo.state["candidate"]["body"] = json!(escaped_key);
    rehash(&mut echo);
    assert_eq!(
        escaped_provider.classify(&echo).await.unwrap_err().kind,
        FailureKind::Rejected
    );
    let mut huge = input();
    huge.state = json!("x".repeat(25 * 1024));
    rehash(&mut huge);
    assert_eq!(
        provider.classify(&huge).await.unwrap_err().kind,
        FailureKind::Oversized
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn endpoint_and_configuration_reject_untrusted_destinations() {
    for endpoint in [
        "https://127.0.0.1:9000/v1/systemone",
        "http://localhost:9000/v1/systemone",
        "http://192.0.2.1:9000/v1/systemone",
        "http://user:pass@127.0.0.1:9000/v1/systemone",
        "http://127.0.0.1:9000/v1/systemone?secret=x",
        "http://127.0.0.1:9000/v1/systemone#x",
        "http://127.0.0.1:9000/other",
    ] {
        assert!(
            Provider::for_loopback(Some(KEY.into()), Duration::from_secs(1), endpoint).is_err()
        );
    }
    assert!(Provider::new(Some("bad\ncredential".into()), Duration::from_secs(1)).is_err());
    assert!(Provider::new(None, Duration::from_millis(999)).is_err());
    assert!(Provider::new(None, Duration::from_secs(31)).is_err());
}

#[tokio::test]
async fn redirect_never_forwards_credentials_or_request() {
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    let location = format!(
        "Location: http://{}/v1/systemone\r\n",
        trap.local_addr().unwrap()
    );
    let bytes = response(307, &location, KEY.as_bytes());
    let (endpoint, handle) = server(move |stream| stream.write_all(&bytes).unwrap());
    let provider =
        Provider::for_loopback(Some(KEY.into()), Duration::from_secs(1), &endpoint).unwrap();
    let error = provider.classify(&input()).await.unwrap_err();
    assert_eq!(error.kind, FailureKind::Redirect);
    assert!(!error.retryable);
    assert_eq!(
        trap.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(!serde_json::to_string(&error).unwrap().contains(KEY));
    handle.join().unwrap();
}

#[tokio::test]
async fn http_failures_are_typed_bounded_and_do_not_retain_error_bodies() {
    for (status, kind, retryable) in [
        (401, FailureKind::Authentication, false),
        (403, FailureKind::Authentication, false),
        (400, FailureKind::Rejected, false),
        (408, FailureKind::Timeout, true),
        (429, FailureKind::RateLimited, true),
        (500, FailureKind::Server, true),
        (503, FailureKind::Server, true),
        (529, FailureKind::Server, true),
    ] {
        let bytes = response(status, "Retry-After: 999999\r\n", KEY.as_bytes());
        let (endpoint, handle) = server(move |stream| stream.write_all(&bytes).unwrap());
        let provider =
            Provider::for_loopback(Some(KEY.into()), Duration::from_secs(1), &endpoint).unwrap();
        let error = provider.classify(&input()).await.unwrap_err();
        assert_eq!(error.kind, kind, "status {status}");
        assert_eq!(error.retryable, retryable);
        assert_eq!(
            error.retry_after_secs,
            if retryable { Some(300) } else { None }
        );
        assert!(error.usage.is_none());
        assert!(!serde_json::to_string(&error).unwrap().contains(KEY));
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn malformed_typed_answers_preserve_known_usage_without_accepting_advice() {
    let mut cases = Vec::new();
    for (pointer, value) in [
        ("/answers/category/type", json!("noul")),
        ("/answers/durable/type", json!("choice")),
        ("/answers/category/choice", json!("lesson")),
        ("/answers/category/confidence", json!(1.1)),
        ("/answers/durable/noul", json!(-0.1)),
        ("/answers/category/probabilities/decision", json!(0.5)),
        ("/model", json!("jev-1.14.0")),
        ("/model", json!("")),
    ] {
        let mut value_root = answer();
        *value_root.pointer_mut(pointer).unwrap() = value;
        cases.push(value_root);
    }
    let mut extra = answer();
    extra["answers"]["category"]["probabilities"]["invented"] = json!(0.0);
    cases.push(extra);
    let mut missing = answer();
    missing["answers"]["category"]["probabilities"]
        .as_object_mut()
        .unwrap()
        .remove("unknown");
    cases.push(missing);
    for value in cases {
        let (endpoint, handle) = json_server(value);
        let provider =
            Provider::for_loopback(Some(KEY.into()), Duration::from_secs(1), &endpoint).unwrap();
        let error = provider.classify(&input()).await.unwrap_err();
        assert_eq!(error.kind, FailureKind::InvalidResponse);
        assert_eq!(error.usage.unwrap().input_tokens, 83);
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn malformed_json_or_usage_never_fabricates_token_counts() {
    let mut cases = vec![b"{not-json".to_vec()];
    for usage in [
        Value::Null,
        json!({"input_tokens":-1,"output_tokens":2}),
        json!({"input_tokens":1.5,"output_tokens":2}),
        json!({"input_tokens":u64::MAX,"output_tokens":2}),
    ] {
        let mut value = answer();
        value["usage"] = usage;
        cases.push(serde_json::to_vec(&value).unwrap());
    }
    for body in cases {
        let bytes = response(200, "", &body);
        let (endpoint, handle) = server(move |stream| stream.write_all(&bytes).unwrap());
        let provider =
            Provider::for_loopback(Some(KEY.into()), Duration::from_secs(1), &endpoint).unwrap();
        let error = provider.classify(&input()).await.unwrap_err();
        assert_eq!(error.kind, FailureKind::InvalidResponse);
        assert!(error.usage.is_none());
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn oversized_fixed_and_chunked_responses_are_rejected() {
    for chunked in [false, true] {
        let body = vec![b'x'; MAX_RESPONSE_BYTES + 1];
        let bytes = if chunked {
            let mut bytes = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",body.len()).into_bytes();
            bytes.extend(body);
            bytes.extend_from_slice(b"\r\n0\r\n\r\n");
            bytes
        } else {
            response(200, "", &body)
        };
        let (endpoint, handle) = server(move |stream| {
            let _ = stream.write_all(&bytes);
        });
        let provider =
            Provider::for_loopback(Some(KEY.into()), Duration::from_secs(1), &endpoint).unwrap();
        assert_eq!(
            provider.classify(&input()).await.unwrap_err().kind,
            FailureKind::Oversized
        );
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn timeout_covers_response_body_not_only_headers() {
    let (endpoint, handle) = server(move |stream| {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{")
            .unwrap();
        thread::sleep(Duration::from_millis(1400));
        let _ = stream.write_all(b"}");
    });
    let provider =
        Provider::for_loopback(Some(KEY.into()), Duration::from_secs(1), &endpoint).unwrap();
    let error = provider.classify(&input()).await.unwrap_err();
    assert_eq!(error.kind, FailureKind::Timeout);
    assert!(error.retryable);
    assert!(error.usage.is_none());
    handle.join().unwrap();
}
