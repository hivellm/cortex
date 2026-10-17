//! Exercise the binary's factory and background-task gates, not only its router.

use cortex_config::{Config, RuntimeProfile};
use reqwest::{Client, Method, StatusCode};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::ErrorKind,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const REPO: &str = "daily-process-test";
const MEMORY_ID: &str = "daily-process-decision";
const MEMORY_BODY: &str = "dailyprocesstoken exact durable decision";
const STARTUP_LIMIT: Duration = Duration::from_secs(10);

struct ApiProcess {
    child: Child,
    url: String,
    log: PathBuf,
}

impl ApiProcess {
    async fn start(root: &Path, config: &Path, sentinel_url: &str, run: &str) -> Self {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        let log = root.join(format!("api-{run}.log"));
        let output = File::create(&log).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex-api"));
        command.env_clear();
        // Keep only executable/DLL bootstrap variables, never operator settings or secrets.
        for name in ["PATH", "SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for name in [
            "VECTORIZER_URL",
            "NEXUS_URL",
            "CORTEX_ADAPTER_ADMIN_URL",
            "CORTEX_CLASSIFIER_WORKER_URL",
            "CORTEX_EMBEDDER_WORKER_URL",
            "CORTEX_FULLTEXT_WORKER_URL",
            "CORTEX_GRAPH_WORKER_URL",
        ] {
            command.env(name, sentinel_url);
        }
        command
            .current_dir(root)
            .env("CORTEX_CONFIG_FILE", config)
            .env("CORTEX_RUNTIME_PROFILE", "daily")
            .env("RUST_LOG", "info")
            .arg("--bind")
            .arg(address.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output));
        drop(reservation);
        let started = Instant::now();
        let mut process = Self {
            child: command.spawn().expect("spawn isolated cortex-api binary"),
            url: format!("http://{address}"),
            log,
        };
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();
        loop {
            if let Some(exit) = process.child.try_wait().unwrap() {
                panic!(
                    "cortex-api exited during startup: {exit}\n{}",
                    process.logs()
                );
            }
            if client
                .get(format!("{}/healthz", process.url))
                .send()
                .await
                .is_ok_and(|response| response.status() == StatusCode::OK)
            {
                assert!(
                    started.elapsed() <= STARTUP_LIMIT,
                    "daily startup exceeded {STARTUP_LIMIT:?}\n{}",
                    process.logs()
                );
                return process;
            }
            assert!(
                started.elapsed() < STARTUP_LIMIT,
                "daily startup exceeded {STARTUP_LIMIT:?}\n{}",
                process.logs()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn logs(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for ApiProcess {
    fn drop(&mut self) {
        // Only this test's child is stopped, including during assertion unwinding.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_full_config(root: &Path, archive: &Path, sentinel_url: &str) -> PathBuf {
    let mut config = Config::default();
    assert_eq!(config.runtime.profile, RuntimeProfile::Full);
    config.ingestion.home = Some(root.to_string_lossy().into_owned());
    config.ingestion.metadata_db =
        Some(root.join("metadata.sqlite").to_string_lossy().into_owned());
    config.ingestion.archive_root = Some(archive.to_string_lossy().into_owned());
    config.ingestion.archive_incremental_enabled = false;
    config.ingestion.cas_db = Some(root.join("cas.sqlite").to_string_lossy().into_owned());
    config.ingestion.pruner_status_file =
        Some(root.join("pruner.json").to_string_lossy().into_owned());
    config.ingestion.archive_watcher_urls = Some(sentinel_url.into());
    config.ingestion.ingestion_url = Some(sentinel_url.into());
    config.ingestion.synap_url = Some(sentinel_url.into());
    config.embedder.vectorizer_url = Some(sentinel_url.into());
    config.embedder.synap_url = sentinel_url.into();
    config.meili.meili_url = Some(sentinel_url.into());
    config.meili.synap_url = sentinel_url.into();
    config.nexus.nexus_url = Some(sentinel_url.into());
    config.nexus.synap_url = sentinel_url.into();
    config.classifier.health_url = Some(sentinel_url.into());
    config.classifier.synap_url = Some(sentinel_url.into());
    config.reranker.enabled = true;
    config.reranker.endpoint = Some(sentinel_url.into());
    config.dashboard.synap_url = Some(sentinel_url.into());
    config.dashboard.api_url = Some(sentinel_url.into());
    config.dashboard.api_keys_db =
        Some(root.join("api-keys.sqlite").to_string_lossy().into_owned());
    config.dashboard.relevance_config_path =
        Some(root.join("relevance.toml").to_string_lossy().into_owned());
    config.dashboard.watch = true;
    config.dashboard.memory_tail = true;
    config.dashboard.query_rewriter = Some("sonnet".into());
    config.dashboard.coverage_slugs = Some(REPO.into());
    config.dashboard.archive_refresh_secs = 1;
    config.dashboard.meili_refresh_secs = 1;
    config.canary.enabled = true;
    config.canary.interval_secs = 10;
    config.canary.pre_thinking_health_secs = 10;
    config.rulebook.root = Some(root.join(".rulebook").to_string_lossy().into_owned());
    config.rulebook.roots = None;
    let path = root.join("cortex.toml");
    fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    path
}

async fn request(
    client: &Client,
    process: &ApiProcess,
    method: Method,
    path: &str,
    payload: Value,
    expected: StatusCode,
) -> Value {
    let response = client
        .request(method, format!("{}{path}", process.url))
        .json(&payload)
        .send()
        .await
        .unwrap_or_else(|error| panic!("{path}: {error}\n{}", process.logs()));
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, expected, "{path}: {body}\n{}", process.logs());
    serde_json::from_str(&body).unwrap_or_else(|_| json!(body))
}

async fn assert_daily_status(client: &Client, process: &ApiProcess) {
    for path in ["/healthz", "/v1/health"] {
        request(
            client,
            process,
            Method::GET,
            path,
            Value::Null,
            StatusCode::OK,
        )
        .await;
    }
    let status = request(
        client,
        process,
        Method::GET,
        "/v1/status",
        Value::Null,
        StatusCode::OK,
    )
    .await;
    assert_eq!(status["runtime_profile"], "daily");
    assert_eq!(status["pid"], process.child.id());
    assert_eq!(status["capabilities"]["daily_memory"]["state"], "ready");
    assert_eq!(
        status["capabilities"]["claude_adapter"]["state"],
        "unprobed"
    );
    assert_eq!(
        status["capabilities"]["daily_memory"]["delivery_consumption"],
        "not_observed"
    );
    for capability in [
        "bulk_indexers",
        "bulk_monitoring",
        "scheduled_maintenance",
        "query_enrichment",
    ] {
        assert_eq!(
            status["capabilities"][capability]["state"],
            "disabled_by_profile"
        );
    }
    assert_eq!(status["archive"]["enabled"], false);
    assert_eq!(status["archive"]["phase"], "disabled");
    assert!(status["archive"]["last_poll"].is_null());
    assert_eq!(status["archive"]["snapshot_hits"], 0);
    assert!(!status["indexed_repos"]
        .as_array()
        .unwrap()
        .iter()
        .any(|repo| repo == "archive-only-repo"));
}

fn assert_durable_state(root: &Path, delivery_id: &str) {
    let database = Connection::open_with_flags(
        root.join("metadata.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let body: String = database
        .query_row(
            "SELECT body FROM daily_memory_records WHERE id=?1",
            [MEMORY_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(body, MEMORY_BODY);
    let repo: String = database
        .query_row(
            "SELECT repo FROM daily_memory_deliveries WHERE id=?1",
            [delivery_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repo, REPO);
    let helpful: bool = database
        .query_row(
            "SELECT helpful FROM pre_thinking_feedback WHERE query_id=?1",
            [delivery_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(helpful);
    let jobs: i64 = database
        .query_row("SELECT COUNT(*) FROM cron_jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(jobs, 0, "daily startup must not seed maintenance jobs");
    let archive_tables: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name GLOB 'archive_index_*'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        archive_tables, 0,
        "an archive root alone must not open the archive index"
    );
}

#[tokio::test]
async fn daily_binary_starts_offline_and_preserves_memory_without_bulk_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let sentinel = TcpListener::bind("127.0.0.1:0").unwrap();
    sentinel.set_nonblocking(true).unwrap();
    let sentinel_url = format!("http://{}", sentinel.local_addr().unwrap());
    let archive = root.join("archive");
    let partition = archive.join("events/year=2020/month=01/day=01");
    fs::create_dir_all(&partition).unwrap();
    let fixture = json!({
        "event_id":"archive-must-stay-cold", "schema_version":"1",
        "occurred_at":"2020-01-01T00:00:00Z", "session_id":"archive-session",
        "stream":"live", "tool":"test", "kind":"turn",
        "context":{"repo":"archive-only-repo", "platform":"test", "extras":{}},
        "payload":{"user_message":"archive sentinel content"}, "content_hash":"fixture-hash"
    });
    let archive_bytes = zstd::encode_all(format!("{fixture}\n").as_bytes(), 1).unwrap();
    let archive_file = partition.join("raw-00000.parquet");
    fs::write(&archive_file, &archive_bytes).unwrap();
    let config = write_full_config(root, &archive, &sentinel_url);
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let process = ApiProcess::start(root, &config, &sentinel_url, "first").await;
    assert_daily_status(&client, &process).await;
    let captured = request(
        &client,
        &process,
        Method::POST,
        "/v1/ingest",
        json!({
            "repo":REPO, "kind":"decision", "body":MEMORY_BODY, "memory_id":MEMORY_ID
        }),
        StatusCode::ACCEPTED,
    )
    .await;
    assert_eq!(captured["durability"], "local_committed");
    assert_eq!(captured["index_complete"], false);
    assert!(captured["indexed_at"].is_null());
    let delivery = request(
        &client,
        &process,
        Method::POST,
        "/v1/memory/query",
        json!({
            "repo":REPO, "query":"dailyprocesstoken"
        }),
        StatusCode::OK,
    )
    .await;
    assert_eq!(delivery["memories"][0]["id"], MEMORY_ID);
    assert!(delivery["context"].as_str().unwrap().contains(MEMORY_BODY));
    let delivery_id = delivery["delivery_id"].as_str().unwrap();
    request(
        &client,
        &process,
        Method::POST,
        "/v1/pre-thinking/feedback",
        json!({
            "repo":REPO, "query_id":delivery_id, "helpful":true
        }),
        StatusCode::OK,
    )
    .await;
    for (method, path) in [
        (Method::POST, "/v1/search/keyword"),
        (Method::POST, "/v1/search/vector"),
        (Method::POST, "/v1/search/graph"),
        (Method::POST, "/v1/search/events"),
        (Method::POST, "/v1/topic-cards/search"),
        (Method::POST, "/v1/decisions/search"),
        (Method::POST, "/v1/laws/violations"),
        (Method::GET, "/v1/health/coverage"),
        (Method::POST, "/v1/admin/forget"),
        (Method::GET, "/v1/consolidations/recent"),
        (Method::GET, "/v1/timeline/daily-process-test"),
        (Method::GET, "/v1/sessions/test/timeline"),
    ] {
        let response = request(
            &client,
            &process,
            method,
            path,
            json!({}),
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;
        assert_eq!(
            response["reason"], "disabled_by_profile",
            "{path}: {response}"
        );
    }
    // Cross the configured refresh interval to catch accidental background startup.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_daily_status(&client, &process).await;
    drop(process);
    assert_durable_state(root, delivery_id);
    let restarted = ApiProcess::start(root, &config, &sentinel_url, "restarted").await;
    assert_daily_status(&client, &restarted).await;
    let recovered = request(
        &client,
        &restarted,
        Method::POST,
        "/v1/memory/query",
        json!({
            "repo":REPO, "query":"dailyprocesstoken"
        }),
        StatusCode::OK,
    )
    .await;
    assert_eq!(recovered["memories"][0]["id"], MEMORY_ID);
    assert!(recovered["context"].as_str().unwrap().contains(MEMORY_BODY));
    drop(restarted);
    assert_durable_state(root, delivery_id);
    assert_eq!(fs::read(archive_file).unwrap(), archive_bytes);
    // A successful TCP handshake remains queued even if its client has closed.
    assert_eq!(
        sentinel.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock,
        "daily process contacted an optional backend"
    );
}
