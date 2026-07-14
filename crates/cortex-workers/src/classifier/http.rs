//! Phase28 (retrieval-eval-gate-live §3) — `POST /v1/classify`
//! admin route for the classifier worker.
//!
//! The cortex-eval `classification` suite posts one golden envelope
//! (`{"tool": …, "kind": …, "payload": …}`) per row and expects
//! `{"kind": "<snake_case>"}` back. Before this route existed the
//! suite's driver targeted a path no worker ever exposed, so every
//! row degraded to `Unknown` and the suite could never produce its
//! first real measurement (the `cdc-baseline-v1.json` placeholder).
//!
//! Kind derivation is the same contract ingestion applies: the
//! envelope's `kind` string deserialises into [`Kind`] via serde
//! (snake_case). The [`StaticClassifier`] then enriches the payload
//! (refinement, topics, severity) exactly like the deployed
//! static-mode worker, so the response reflects the production
//! static path rather than a bespoke eval shim. An unknown kind
//! string returns `400` — the eval driver counts those as `Unknown`
//! predictions, which is the honest failure signal.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{routing::post, Json, Router};
use cortex_core::events::Kind;
use serde_json::{json, Value};

use super::statics::StaticClassifier;
use super::types::{Classifier, EnrichmentInput};

/// Build the classifier worker's extra admin router.
pub fn classify_router() -> Router {
    Router::new().route("/v1/classify", post(handle_classify))
}

async fn handle_classify(Json(envelope): Json<Value>) -> Response {
    let kind_str = envelope
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let kind: Kind = match serde_json::from_value(Value::String(kind_str.clone())) {
        Ok(k) => k,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "reason": "bad_input",
                    "detail": format!("unknown envelope kind `{kind_str}`"),
                })),
            )
                .into_response();
        }
    };
    let payload = envelope.get("payload").cloned().unwrap_or(Value::Null);
    let input = EnrichmentInput {
        event_id: "eval-classify".to_string(),
        kind,
        content_hash: String::new(),
        redacted_payload: payload,
        context_repo: None,
    };
    match StaticClassifier::new().classify_batch(&[input]).await {
        Ok(outputs) => {
            let out = outputs.first();
            // Round-trip the Kind through serde so the wire label is
            // exactly the canonical snake_case string.
            let kind_label = serde_json::to_value(kind)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or(kind_str);
            (
                StatusCode::OK,
                Json(json!({
                    "kind": kind_label,
                    "kind_refinement": out.and_then(|o| o.kind_refinement.clone()),
                    "topics": out.map(|o| o.topics.clone()).unwrap_or_default(),
                    "severity": out.map(|o| format!("{:?}", o.severity).to_lowercase()),
                    "source": "static",
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "reason": "classifier_error", "detail": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn post_classify(body: Value) -> (StatusCode, Value) {
        let resp = classify_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/classify")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    #[tokio::test]
    async fn classify_returns_canonical_snake_case_kind() {
        let (status, body) = post_classify(json!({
            "tool": "claude-code",
            "kind": "tool_call",
            "payload": {"tool_name": "Bash", "input": {"command": "cargo check"}}
        }))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["kind"], "tool_call");
        assert_eq!(body["source"], "static");
    }

    #[tokio::test]
    async fn classify_covers_every_kind_variant() {
        for kind in [
            "turn",
            "tool_call",
            "agent_call",
            "memory",
            "decision",
            "analysis",
            "law",
            "law_violation",
            "artifact",
            "knowledge",
            "learning",
            "consolidation",
            "topic_card",
        ] {
            let (status, body) =
                post_classify(json!({"tool": "t", "kind": kind, "payload": {}})).await;
            assert_eq!(status, StatusCode::OK, "kind {kind} must classify");
            assert_eq!(body["kind"], kind, "kind {kind} must round-trip");
        }
    }

    #[tokio::test]
    async fn classify_rejects_unknown_kind_with_400() {
        let (status, body) =
            post_classify(json!({"tool": "t", "kind": "not_a_kind", "payload": {}})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["reason"], "bad_input");
    }
}
