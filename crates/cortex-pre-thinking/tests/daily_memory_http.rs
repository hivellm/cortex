//! Real async HTTP transport contracts with a hermetic loopback server.

use cortex_pre_thinking::{DailyMemoryFn, DailyMemoryRequest, HttpDailyMemory};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn request() -> DailyMemoryRequest {
    DailyMemoryRequest {
        repo: "Exact/Repo".into(),
        query: "sqlite correction".into(),
        branch: Some("main".into()),
        paths: vec!["src/store.rs".into()],
        include_global: false,
        limit: 5,
        budget_bytes: 1400,
    }
}

fn response() -> Value {
    json!({"context":"[memory ADR-42 v1 event source-event] source=src/store.rs\nDurable correction.\n", "memories":[{"id":"ADR-42","version":1,"event_id":"source-event","scope":{"repository":"Exact/Repo"}}],"revision":9,"global_revision":null,"delivery_id":"prepared-42","stage":"prepared"})
}

async fn server(body: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let count = socket.read(&mut chunk).await.unwrap();
            if count == 0 {
                break;
            }
            received.extend_from_slice(&chunk[..count]);
            assert!(received.len() <= 32 * 1024);
            let text = String::from_utf8_lossy(&received);
            if let Some((headers, _)) = text.split_once("\r\n\r\n") {
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if received.len() >= headers.len() + 4 + length {
                    break;
                }
            }
        }
        let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = socket.write_all(header.as_bytes()).await;
        let _ = socket.write_all(&body).await;
        String::from_utf8(received).unwrap()
    });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn http_forwards_bearer_exact_scope_and_prepared_receipt_without_inventing_cache_hit() {
    let (url, task) = server(serde_json::to_vec(&response()).unwrap()).await;
    let transport =
        HttpDailyMemory::new(reqwest::Client::new(), url, Some("test-credential".into()));
    let result = transport.read(request()).await.unwrap();
    assert_eq!(result.delivery_id, "prepared-42");
    assert_eq!(result.revision, 9);
    assert!(!result.cache_hit);
    let received = task.await.unwrap();
    assert!(received
        .to_ascii_lowercase()
        .contains("authorization: bearer test-credential"));
    assert!(received.starts_with("POST /v1/memory/query "));
    let body: Value = serde_json::from_str(received.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["repo"], "Exact/Repo");
    assert_eq!(body["branch"], "main");
    assert_eq!(body["paths"], json!(["src/store.rs"]));
    assert_eq!(body["include_global"], false);
}

#[tokio::test]
async fn http_rejects_stream_over_16k_invalid_json_and_cross_repo_metadata() {
    let mut foreign = response();
    foreign["memories"][0]["scope"]["repository"] = json!("other-repo");
    for body in [
        "x".repeat(16 * 1024 + 1).into_bytes(),
        b"invalid json".to_vec(),
        serde_json::to_vec(&foreign).unwrap(),
    ] {
        let (url, task) = server(body).await;
        let transport = HttpDailyMemory::new(reqwest::Client::new(), url, None);
        assert!(transport.read(request()).await.is_none());
        task.await.unwrap();
    }
}
