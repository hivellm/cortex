//! Loopback-only browser transport. No provider credentials or filesystem operations cross this API.
use crate::{
    classification::{ClassificationQueue, DEFAULT_MODEL},
    AuditListQuery, AuditMetricsQuery, Authority, CaptureRequest, GraphQuery, LocalError,
    LocalStore, RecordKind, Result, RevisionRequest, SearchQuery, Source, TypedLink,
};
use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

mod dashboard_assets {
    include!(concat!(env!("OUT_DIR"), "/dashboard_assets.rs"));
}

#[derive(Clone)]
struct WebState {
    store: LocalStore,
    host: String,
    origin: String,
    token: Arc<str>,
}

/// Build the same authenticated router used by `serve`; the caller must bind loopback.
pub fn router(store: LocalStore, port: u16, token: String) -> Result<Router> {
    if port == 0 || token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(LocalError::Invalid(
            "invalid local web configuration".into(),
        ));
    }
    let host = format!("127.0.0.1:{port}");
    let state = WebState {
        store,
        origin: format!("http://{host}"),
        host,
        token: token.into(),
    };
    Ok(Router::new()
        .route("/", get(asset))
        .fallback(asset)
        .route("/api/repositories", post(repositories))
        .route("/api/browse", post(browse))
        .route("/api/recent", post(recent))
        .route("/api/search", post(search))
        .route("/api/get", post(record))
        .route("/api/graph", post(graph))
        .route("/api/status", post(status))
        .route("/api/capture", post(capture))
        .route("/api/revise", post(revise))
        .route("/api/queue", post(queue))
        .route("/api/enqueue", post(enqueue))
        .route("/api/audit/list", post(audit_list))
        .route("/api/audit/get", post(audit_get))
        .route("/api/audit/metrics", post(audit_metrics))
        .route("/api/feedback/record", post(feedback_record))
        .route("/api/feedback/get", post(feedback_get))
        .route("/api/feedback/list", post(feedback_list))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state))
}

async fn asset(request: Request) -> Response {
    if request.method() != axum::http::Method::GET && request.method() != axum::http::Method::HEAD {
        return failure(
            StatusCode::METHOD_NOT_ALLOWED,
            "Use GET for dashboard assets",
        );
    }
    match dashboard_assets::ASSETS
        .iter()
        .find(|(path, _, _)| *path == request.uri().path())
    {
        // The HTTP server strips HEAD bodies while retaining the GET length.
        Some((_, mime, bytes)) => ([(header::CONTENT_TYPE, *mime)], *bytes).into_response(),
        None => failure(StatusCode::NOT_FOUND, "Dashboard asset not found"),
    }
}

/// Print a one-session bootstrap URL to stderr; the fragment is never sent over HTTP.
pub async fn serve(store: LocalStore, port: u16, repository: Option<String>) -> Result<()> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let port = listener.local_addr()?.port();
    let mut random = [0_u8; 32];
    getrandom::getrandom(&mut random)
        .map_err(|_| LocalError::Invalid("OS randomness unavailable".into()))?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let app = router(store, port, token.clone())?;
    let mut url = reqwest::Url::parse(&format!("http://127.0.0.1:{port}/"))
        .map_err(|_| LocalError::Invalid("invalid local web address".into()))?;
    url.query_pairs_mut().append_pair("token", &token);
    if let Some(repository) = repository {
        crate::files::validate_scope(&repository)?;
        url.query_pairs_mut().append_pair("repository", &repository);
    }
    let fragment = url.query().map(str::to_owned);
    url.set_query(None);
    url.set_fragment(fragment.as_deref());
    eprintln!("Cortex local: {url}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn matches_token(actual: &[u8], expected: &[u8]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .fold(0_u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

async fn guard(State(state): State<WebState>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host_ok = headers.get_all(header::HOST).iter().count() == 1
        && headers
            .get(header::HOST)
            .is_some_and(|value| value.as_bytes() == state.host.as_bytes());
    let origin = headers.get(header::ORIGIN);
    let origin_ok = headers.get_all(header::ORIGIN).iter().count() <= 1
        && origin.map_or(true, |value| value.as_bytes() == state.origin.as_bytes());
    let api = request.uri().path().starts_with("/api/");
    let authorized = headers.get_all(header::AUTHORIZATION).iter().count() == 1
        && headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.as_bytes().strip_prefix(b"Bearer "))
            .is_some_and(|value| matches_token(value, state.token.as_bytes()));
    let mut response = if !host_ok || !origin_ok || (api && origin.is_none()) {
        failure(
            StatusCode::FORBIDDEN,
            "Local host and same-origin request required",
        )
    } else if api && !authorized {
        failure(
            StatusCode::UNAUTHORIZED,
            "Open the startup URL to connect this tab",
        )
    } else if api && request.method() != axum::http::Method::POST {
        failure(StatusCode::METHOD_NOT_ALLOWED, "Use a JSON POST request")
    } else {
        next.run(request).await
    };
    for (key, value) in [
        ("content-security-policy", "default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"),
        ("x-content-type-options", "nosniff"),
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        response.headers_mut().insert(key, HeaderValue::from_static(value));
    }
    response
}

fn failure(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

async fn work<T: serde::Serialize + Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(operation).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(LocalError::Invalid(_))) => {
            failure(StatusCode::BAD_REQUEST, "Invalid record or query")
        }
        Ok(Err(LocalError::Conflict(_))) => {
            failure(StatusCode::CONFLICT, "Record changed; reload before saving")
        }
        Ok(Err(LocalError::NotFound(_))) => {
            failure(StatusCode::NOT_FOUND, "Record not found in this repository")
        }
        _ => failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Local operation failed; inspect the store with the CLI",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    repository: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryRequest {}

async fn repositories(
    State(state): State<WebState>,
    Json(_query): Json<InventoryRequest>,
) -> Response {
    work(move || state.store.repositories()).await
}

async fn browse(State(state): State<WebState>, Json(query): Json<crate::BrowseQuery>) -> Response {
    work(move || state.store.browse(query)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recent {
    repository: String,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    include_inactive: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueRequest {
    repository: String,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    30
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetRecord {
    repository: String,
    id: String,
    revision: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetAudit {
    repository: String,
    id: String,
}

async fn audit_list(State(state): State<WebState>, Json(query): Json<AuditListQuery>) -> Response {
    work(move || state.store.list_audit(query)).await
}

async fn feedback_record(
    State(state): State<WebState>,
    Json(request): Json<crate::feedback::FeedbackRequest>,
) -> Response {
    work(move || state.store.record_feedback(request)).await
}
async fn feedback_get(State(state): State<WebState>, Json(query): Json<GetAudit>) -> Response {
    work(move || {
        state
            .store
            .get_feedback(&query.repository, &query.id)?
            .ok_or(LocalError::NotFound(query.id))
    })
    .await
}
async fn feedback_list(
    State(state): State<WebState>,
    Json(query): Json<crate::feedback::FeedbackListQuery>,
) -> Response {
    work(move || state.store.list_feedback(query)).await
}

async fn audit_get(State(state): State<WebState>, Json(query): Json<GetAudit>) -> Response {
    work(move || {
        state
            .store
            .get_audit(&query.repository, &query.id)?
            .ok_or(LocalError::NotFound(query.id))
    })
    .await
}

async fn audit_metrics(
    State(state): State<WebState>,
    Json(query): Json<AuditMetricsQuery>,
) -> Response {
    work(move || state.store.audit_metrics(query)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revise {
    repository: String,
    id: String,
    expected_revision: u64,
    request: RevisionRequest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enqueue {
    repository: String,
    id: String,
    model: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capture {
    repository: String,
    title: String,
    body: String,
    #[serde(default)]
    kind: RecordKind,
    #[serde(default)]
    authority: Authority,
    #[serde(default)]
    links: Vec<TypedLink>,
    #[serde(default)]
    metadata: Value,
}

async fn recent(State(state): State<WebState>, Json(query): Json<Recent>) -> Response {
    work(move || {
        state
            .store
            .recent(&query.repository, query.limit, query.include_inactive)
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebSearch {
    repository: String,
    text: String,
    limit: usize,
    #[serde(default)]
    include_inactive: bool,
    #[serde(default)]
    feedback: bool,
    #[serde(default, deserialize_with = "present_search_time")]
    as_of: Option<String>,
}
fn present_search_time<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}
async fn search(State(state): State<WebState>, Json(query): Json<WebSearch>) -> Response {
    work(move || {
        if !query.feedback && query.as_of.is_some() {
            return Err(LocalError::Invalid("as_of requires feedback=true".into()));
        }
        state.store.search_with_feedback(
            SearchQuery {
                repository: query.repository,
                text: query.text,
                limit: query.limit,
                include_inactive: query.include_inactive,
            },
            crate::RankingOptions {
                enabled: query.feedback,
                as_of: query.as_of,
            },
        )
    })
    .await
}
async fn record(State(state): State<WebState>, Json(query): Json<GetRecord>) -> Response {
    work(move || {
        let record = match query.revision {
            Some(revision) => state
                .store
                .get_revision(&query.repository, &query.id, revision)?,
            None => state.store.get(&query.repository, &query.id)?,
        };
        record.ok_or_else(|| LocalError::NotFound(query.id))
    })
    .await
}
async fn graph(State(state): State<WebState>, Json(query): Json<GraphQuery>) -> Response {
    work(move || state.store.graph(query)).await
}
async fn status(State(state): State<WebState>, Json(query): Json<Scope>) -> Response {
    work(move || {
        crate::files::validate_scope(&query.repository)?;
        state.store.status(Some(&query.repository))
    })
    .await
}
async fn capture(State(state): State<WebState>, Json(query): Json<Capture>) -> Response {
    work(move || {
        state.store.capture(CaptureRequest {
            repository: query.repository,
            title: query.title,
            body: query.body,
            kind: query.kind,
            authority: query.authority,
            source: Source {
                kind: "local_web".into(),
                locator: "user".into(),
                ..Source::default()
            },
            links: query.links,
            metadata: query.metadata,
            ..CaptureRequest::default()
        })
    })
    .await
}
async fn revise(State(state): State<WebState>, Json(query): Json<Revise>) -> Response {
    work(move || {
        state.store.revise(
            &query.repository,
            &query.id,
            query.expected_revision,
            query.request,
        )
    })
    .await
}
async fn queue(State(state): State<WebState>, Json(query): Json<QueueRequest>) -> Response {
    work(move || ClassificationQueue::new(state.store).list(&query.repository, query.limit)).await
}
async fn enqueue(State(state): State<WebState>, Json(query): Json<Enqueue>) -> Response {
    work(move || {
        ClassificationQueue::new(state.store).enqueue(
            &query.repository,
            &query.id,
            query.model.as_deref().unwrap_or(DEFAULT_MODEL),
        )
    })
    .await
}
