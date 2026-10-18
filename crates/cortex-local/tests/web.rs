use cortex_local::{web, Authority, CaptureRequest, LocalStore, RecordKind};
use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn audit_routes_preserve_core_parity_and_historical_literal_evidence() {
    let app = App::new().await;
    let dangerous = "<script>alert('audit')</script>";
    let original = app.capture(dangerous).await;
    let prepared=cortex_local::mcp::call(&app.store,"cortex_pre_thinking",json!({"repository":"Cortex","attribution":{"client":"codex","session_id":"s","turn_id":"t"}}),None).unwrap();
    let id = prepared["delivery_id"].as_str().unwrap();
    let receipt = app.store.get_audit("Cortex", id).unwrap().unwrap();
    let event = app
        .store
        .capture(CaptureRequest {
            id: Some("literal-stop".into()),
            repository: "Cortex".into(),
            kind: RecordKind::Event,
            body: format!(
                "{dangerous} [{}@1 {}]",
                original["id"].as_str().unwrap(),
                original["content_sha256"].as_str().unwrap()
            ),
            source: cortex_local::Source {
                kind: "lifecycle_hook".into(),
                ..Default::default()
            },
            metadata: json!({"event":"Stop","client":"codex","session_id":"s","turn_id":"t"}),
            ..Default::default()
        })
        .unwrap()
        .record;
    let observed = app.store.observe_event(&event).unwrap().record;
    app.store
        .revise(
            "Cortex",
            original["id"].as_str().unwrap(),
            1,
            cortex_local::RevisionRequest {
                title: "revised".into(),
                body: "new body".into(),
                authority: Authority::Accepted,
                links: Vec::new(),
                metadata: Value::Null,
            },
        )
        .unwrap();
    let list_args = json!({"repository":"Cortex","limit":20});
    let expected = app
        .store
        .list_audit(serde_json::from_value(list_args.clone()).unwrap())
        .unwrap();
    assert_eq!(
        app.json("audit/list", list_args).await,
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(
        app.json("audit/get", json!({"repository":"Cortex","id":id}))
            .await,
        serde_json::to_value(&receipt).unwrap()
    );
    let observation = app
        .json("audit/get", json!({"repository":"Cortex","id":observed.id}))
        .await;
    assert_eq!(observation["evidence_label"], "captured_citation");
    assert_eq!(
        observation["payload"]["content"]["evidence"][0]["revision"],
        1
    );
    let metrics_args = json!({"repository":"Cortex","limit":10000});
    let expected = app
        .store
        .audit_metrics(serde_json::from_value(metrics_args.clone()).unwrap())
        .unwrap();
    assert_eq!(
        app.json("audit/metrics", metrics_args).await,
        serde_json::to_value(expected).unwrap()
    );
    let response = app
        .post(
            "get",
            json!({"repository":"Cortex","id":event.id,"revision":1}),
        )
        .await;
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    assert!(response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("script-src 'self'"));
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["body"], event.body);
    let historical = app
        .json(
            "get",
            json!({"repository":"Cortex","id":original["id"],"revision":1}),
        )
        .await;
    assert_eq!(historical["title"], dangerous);
    assert_eq!(historical["content_sha256"], original["content_sha256"]);
}

#[tokio::test]
async fn audit_routes_enforce_exact_scope_bounds_and_strict_json() {
    let app = App::new().await;
    let prepared = cortex_local::mcp::call(
        &app.store,
        "cortex_pre_thinking",
        json!({"repository":"Cortex"}),
        None,
    )
    .unwrap();
    let id = prepared["delivery_id"].as_str().unwrap();
    assert_eq!(
        app.post("audit/get", json!({"repository":"cortex","id":id}))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.post("audit/get", json!({"repository":"Cortex","id":"missing"}))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(app
        .json("audit/list", json!({"repository":"cortex","limit":20}))
        .await["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        app.json(
            "audit/metrics",
            json!({"repository":"cortex","limit":10000})
        )
        .await["matching_records"],
        0
    );
    for (endpoint, args, status) in [
        (
            "audit/list",
            json!({"repository":"Cortex","limit":0}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "audit/list",
            json!({"repository":"Cortex","limit":101}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "audit/list",
            json!({"repository":"Cortex","limit":20,"before":"bad"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "audit/list",
            json!({"repository":"cortex","limit":20,"cursor":id}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "audit/metrics",
            json!({"repository":"Cortex","limit":10001}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "audit/metrics",
            json!({"repository":"","limit":10}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "audit/get",
            json!({"repository":"Cortex","id":id,"revision":1}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "audit/list",
            json!({"repository":"Cortex","limit":20,"extra":true}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "audit/metrics",
            json!({"repository":"Cortex","limit":20,"extra":true}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "audit/list",
            json!({"limit":20}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        assert_eq!(
            app.post(endpoint, args).await.status(),
            status,
            "{endpoint}"
        );
    }
}

#[tokio::test]
async fn every_audit_route_requires_same_origin_host_and_bearer() {
    let app = App::new().await;
    for endpoint in ["audit/list", "audit/get", "audit/metrics"] {
        for (host, origin, token, status) in [
            (
                None,
                Some(app.url.as_str()),
                Some("wrong"),
                StatusCode::UNAUTHORIZED,
            ),
            (None, None, Some(TOKEN), StatusCode::FORBIDDEN),
            (
                None,
                Some("https://foreign.invalid"),
                Some(TOKEN),
                StatusCode::FORBIDDEN,
            ),
            (
                Some("foreign.invalid"),
                Some(app.url.as_str()),
                Some(TOKEN),
                StatusCode::FORBIDDEN,
            ),
        ] {
            let mut request = app.client.post(format!("{}/api/{endpoint}", app.url));
            if let Some(host) = host {
                request = request.header("Host", host);
            }
            if let Some(origin) = origin {
                request = request.header("Origin", origin);
            }
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let response = request
                .json(&json!({"repository":"Cortex"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{endpoint}");
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
    }
}

struct App {
    _home: tempfile::TempDir,
    store: LocalStore,
    url: String,
    client: Client,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for App {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl App {
    async fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let store = LocalStore::open(home.path()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = web::router(store.clone(), port, TOKEN.into()).unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            _home: home,
            store,
            url: format!("http://127.0.0.1:{port}"),
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            task,
        }
    }
    async fn post(&self, endpoint: &str, body: Value) -> Response {
        self.client
            .post(format!("{}/api/{endpoint}", self.url))
            .header("Origin", &self.url)
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
            .await
            .unwrap()
    }
    async fn json(&self, endpoint: &str, body: Value) -> Value {
        let response = self.post(endpoint, body).await;
        assert_eq!(response.status(), StatusCode::OK, "{endpoint}");
        response.json().await.unwrap()
    }
    async fn capture(&self, title: &str) -> Value {
        self.json("capture", json!({"repository":"Cortex","title":title,"body":"Remember the reason.","kind":"decision","authority":"accepted"})).await["record"].clone()
    }
}

#[tokio::test]
async fn dashboard_inventory_and_browse_preserve_core_parity_and_cursor_scope() {
    let app = App::new().await;
    app.capture("First dashboard decision").await;
    app.capture("Second dashboard decision").await;
    app.json("capture", json!({"repository":"cortex","title":"Other exact scope","body":"Foreign decision","kind":"decision","authority":"accepted"})).await;

    let inventory = app.json("repositories", json!({})).await;
    assert_eq!(
        inventory,
        serde_json::to_value(app.store.repositories().unwrap()).unwrap()
    );
    assert_eq!(inventory["repositories"][0]["repository"], "Cortex");
    assert_eq!(inventory["repositories"][0]["records"], 2);
    assert_eq!(inventory["repositories"][1]["repository"], "cortex");
    assert_eq!(inventory["repositories"][1]["records"], 1);

    let mut request = json!({"repository":"Cortex","kind":"decision","limit":1});
    let first = app.json("browse", request.clone()).await;
    let core = app
        .store
        .browse(serde_json::from_value(request.clone()).unwrap())
        .unwrap();
    assert_eq!(first, serde_json::to_value(core).unwrap());
    assert_eq!(first["records"].as_array().unwrap().len(), 1);
    let cursor = first["next_cursor"].as_str().unwrap();
    request["cursor"] = json!(cursor);
    let second = app.json("browse", request.clone()).await;
    let core = app
        .store
        .browse(serde_json::from_value(request.clone()).unwrap())
        .unwrap();
    assert_eq!(second, serde_json::to_value(core).unwrap());
    assert_eq!(second["records"].as_array().unwrap().len(), 1);
    assert_ne!(first["records"][0]["id"], second["records"][0]["id"]);
    assert!(second["next_cursor"].is_null());
    for page in [&first, &second] {
        assert_eq!(page["records"][0]["repository"], "Cortex");
        assert_eq!(page["records"][0]["kind"], "decision");
    }
    let foreign = app
        .json("browse", json!({"repository":"cortex","kind":"decision"}))
        .await;
    assert_eq!(foreign["records"].as_array().unwrap().len(), 1);
    assert_eq!(foreign["records"][0]["title"], "Other exact scope");
    assert!(app
        .json("browse", json!({"repository":"Cortex","kind":"event"}))
        .await["records"]
        .as_array()
        .unwrap()
        .is_empty());

    for (field, value) in [
        ("repository", json!("cortex")),
        ("kind", json!("event")),
        ("include_inactive", json!(true)),
        ("cursor", json!("malformed")),
    ] {
        let mut invalid = request.clone();
        invalid[field] = value;
        assert_eq!(
            app.post("browse", invalid).await.status(),
            StatusCode::BAD_REQUEST,
            "{field}"
        );
    }
    request["unknown"] = json!(true);
    assert_eq!(
        app.post("browse", request).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.post("repositories", json!({"repository":"Cortex"}))
            .await
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn dashboard_assets_are_embedded_local_and_hardened() {
    let app = App::new().await;
    let html = app
        .client
        .get(&app.url)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("name=\"cortex-runtime\" content=\"standalone\""));
    assert!(html.contains("id=\"root\""));
    let pattern = regex::Regex::new(r#"(?:src|href)="(/assets/[^\"]+)""#).unwrap();
    let mut paths = vec!["/".to_owned(), "/favicon.svg".to_owned()];
    paths.extend(
        pattern
            .captures_iter(&html)
            .map(|capture| capture[1].to_owned()),
    );
    assert!(paths.iter().any(|path| path.ends_with(".js")));
    assert!(paths.iter().any(|path| path.ends_with(".css")));
    for path in &paths {
        let response = app
            .client
            .get(format!("{}{path}", app.url))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
        let csp = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("script-src 'self';"));
        assert!(csp.contains("style-src-attr 'unsafe-inline';"));
        assert!(!csp.contains("script-src 'self' 'unsafe-inline'"));
        let content_type = response.headers()["content-type"].to_str().unwrap();
        if path.ends_with(".js") {
            assert_eq!(content_type, "text/javascript; charset=utf-8");
        }
        if path.ends_with(".css") {
            assert_eq!(content_type, "text/css; charset=utf-8");
        }
        assert!(response
            .headers()
            .get("access-control-allow-origin")
            .is_none());
        let body = response.text().await.unwrap();
        assert!(!body.contains(TOKEN));
        assert!(!body.is_empty());
        let head = app
            .client
            .head(format!("{}{path}", app.url))
            .send()
            .await
            .unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(
            head.headers()["content-length"].to_str().unwrap(),
            body.len().to_string()
        );
        assert!(head.bytes().await.unwrap().is_empty());
    }
    for path in [
        "/missing",
        "/assets/missing.js",
        "/assets/%2e%2e/Cargo.toml",
        "/app.js",
        "/app.css",
    ] {
        assert_eq!(
            app.client
                .get(format!("{}{path}", app.url))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    assert!(web::router(app.store.clone(), 0, TOKEN.into()).is_err());
    assert!(web::router(app.store.clone(), 1234, "weak".into()).is_err());
}

#[tokio::test]
async fn host_origin_and_bearer_are_required_before_every_api_operation() {
    let app = App::new().await;
    let body = json!({"repository":"Cortex"});
    for endpoint in [
        "repositories",
        "browse",
        "recent",
        "search",
        "get",
        "graph",
        "status",
        "capture",
        "revise",
        "queue",
        "enqueue",
        "audit/list",
        "audit/get",
        "audit/metrics",
        "feedback/record",
        "feedback/get",
        "feedback/list",
    ] {
        let response = app
            .client
            .post(format!("{}/api/{endpoint}", app.url))
            .header("Origin", &app.url)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{endpoint}");
    }
    for origin in [
        "http://evil.example",
        "null",
        "http://127.0.0.1:1",
        "https://127.0.0.1:1",
    ] {
        let response = app
            .client
            .post(format!("{}/api/status", app.url))
            .header("Origin", origin)
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    for host in ["evil.example", "localhost:1234", "127.0.0.1", "127.0.0.1:1"] {
        let response = app
            .client
            .get(format!("{}/", app.url))
            .header("Host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = app
        .client
        .post(format!("{}/api/status", app.url))
        .bearer_auth(TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .client
        .post(format!("{}/api/status", app.url))
        .header("Origin", &app.url)
        .bearer_auth("wrong")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .client
        .get(format!("{}/api/status?token={TOKEN}", app.url))
        .header("Origin", &app.url)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(app.store.status(None).unwrap().records, 0);
}

#[tokio::test]
async fn capture_search_recent_and_sources_preserve_literal_content_and_exact_scope() {
    let app = App::new().await;
    let dangerous = "<img src=x onerror=alert(1)> <script>fetch('/secret')</script>";
    let record = app.capture(dangerous).await;
    assert_eq!(record["title"], dangerous);
    assert_eq!(record["source"]["kind"], "local_web");
    assert_eq!(record["source"]["locator"], "user");
    assert_eq!(record["authority"], "accepted");
    let id = record["id"].as_str().unwrap();
    let fetched = app
        .json("get", json!({"repository":"Cortex","id":id}))
        .await;
    assert_eq!(fetched, record);
    let recent = app
        .json("recent", json!({"repository":"Cortex","limit":10}))
        .await;
    assert_eq!(recent.as_array().unwrap().len(), 1);
    let search = app
        .json(
            "search",
            json!({"repository":"Cortex","text":"Remember","limit":10}),
        )
        .await;
    assert_eq!(search[0]["record"]["id"], id);
    assert_eq!(
        app.json("recent", json!({"repository":"cortex"})).await,
        json!([])
    );
    assert_eq!(
        app.post("get", json!({"repository":"cortex","id":id}))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let counts = app.json("status", json!({"repository":"Cortex"})).await;
    assert_eq!(counts["memories"], 1);
    assert_eq!(app.post("capture", json!({"repository":"Cortex","title":"fake source","body":"text","source":{"locator":"C:/secret"}})).await.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(app.store.status(None).unwrap().records, 1);
}

#[tokio::test]
async fn revisions_conflicts_history_graph_and_immutable_events_use_shared_engine() {
    let app = App::new().await;
    let first = app.capture("Original").await;
    let second = app.capture("Related").await;
    let id = first["id"].as_str().unwrap();
    let revision = json!({"repository":"Cortex","id":id,"expected_revision":1,"request":{"title":"Revised","body":"new evidence","authority":"accepted","links":[{"target_id":second["id"],"relation":"supports"}],"metadata":{"advisory":"retained"}}});
    let revised = app.json("revise", revision.clone()).await;
    assert_eq!(revised["record"]["revision"], 2);
    assert_eq!(
        app.post("revise", revision).await.status(),
        StatusCode::CONFLICT
    );
    let old = app
        .json("get", json!({"repository":"Cortex","id":id,"revision":1}))
        .await;
    assert_eq!(old["title"], "Original");
    assert_eq!(old["source"], revised["record"]["source"]);
    let graph = app
        .json(
            "graph",
            json!({"repository":"Cortex","id":id,"depth":2,"limit":20}),
        )
        .await;
    assert_eq!(graph["records"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"][0]["relation"], "supports");
    let event = app
        .store
        .capture(CaptureRequest {
            repository: "Cortex".into(),
            kind: RecordKind::Event,
            authority: Authority::Observation,
            title: "raw".into(),
            body: "immutable".into(),
            ..Default::default()
        })
        .unwrap()
        .record;
    let response = app.post("revise", json!({"repository":"Cortex","id":event.id,"expected_revision":1,"request":{"title":"changed","body":"changed","authority":"accepted","links":[],"metadata":{}}})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn queue_is_explicit_local_and_cannot_import_repair_or_drain() {
    let app = App::new().await;
    assert_eq!(
        app.post(
            "queue",
            json!({"repository":"Cortex","include_inactive":true})
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let record = app.capture("Queue subject").await;
    assert_eq!(
        app.json("queue", json!({"repository":"Cortex"})).await["jobs"],
        json!([])
    );
    let job = app
        .json("enqueue", json!({"repository":"Cortex","id":record["id"]}))
        .await;
    assert_eq!(job["status"], "pending");
    assert_eq!(job["attempts"], 0);
    assert_eq!(
        app.json("queue", json!({"repository":"Other"})).await["jobs"],
        json!([])
    );
    let fetched = app
        .json("get", json!({"repository":"Cortex","id":record["id"]}))
        .await;
    assert_eq!(fetched, record);
    for endpoint in [
        "drain", "import", "repair", "classify", "../files", "config",
    ] {
        assert_ne!(
            app.post(endpoint, json!({"path":"C:/secret"}))
                .await
                .status(),
            StatusCode::OK
        );
    }
    assert_eq!(
        app.json("queue", json!({"repository":"Cortex"})).await["jobs"][0]["attempts"],
        0
    );
}

#[tokio::test]
async fn malformed_massive_and_non_json_requests_do_not_publish_records() {
    let app = App::new().await;
    for body in ["{", "null", "[]"] {
        let response = app
            .client
            .post(format!("{}/api/capture", app.url))
            .header("Origin", &app.url)
            .bearer_auth(TOKEN)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_client_error());
    }
    let response = app
        .client
        .post(format!("{}/api/capture", app.url))
        .header("Origin", &app.url)
        .bearer_auth(TOKEN)
        .body("repository=Cortex&title=forged")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let response = app
        .client
        .post(format!("{}/api/capture", app.url))
        .header("Origin", &app.url)
        .bearer_auth(TOKEN)
        .header("Content-Type", "application/json")
        .body("x".repeat(2 * 1024 * 1024 + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        app.post("recent", json!({"repository":"Cortex","limit":100000}))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.post("get", json!({"repository":"Cortex","id":"../../secret"}))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.store.status(None).unwrap().records, 0);
}
