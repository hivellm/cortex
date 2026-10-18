use cortex_local::{web, Authority, CaptureRequest, LocalStore, RecordKind};
use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

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
async fn assets_are_local_hardened_and_never_embed_authentication_or_html_sinks() {
    let app = App::new().await;
    for path in ["/", "/app.js", "/app.css", "/favicon.svg", "/missing"] {
        let response = app
            .client
            .get(format!("{}{path}", app.url))
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
        let csp = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(!csp.contains("unsafe-inline"));
        assert!(response
            .headers()
            .get("access-control-allow-origin")
            .is_none());
        let body = response.text().await.unwrap();
        assert!(!body.contains(TOKEN));
        assert!(!body.contains("innerHTML"));
        assert!(!body.contains("insertAdjacentHTML"));
        assert!(!body.contains("eval("));
        if path == "/app.js" {
            assert!(body.contains("textContent"));
            assert!(body.contains("history.replaceState"));
            assert!(body.contains("sessionStorage"));
        }
    }
    assert!(web::router(app.store.clone(), 0, TOKEN.into()).is_err());
    assert!(web::router(app.store.clone(), 1234, "weak".into()).is_err());
}

#[tokio::test]
async fn host_origin_and_bearer_are_required_before_every_api_operation() {
    let app = App::new().await;
    let body = json!({"repository":"Cortex"});
    for endpoint in [
        "recent", "search", "get", "graph", "status", "capture", "revise", "queue", "enqueue",
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
