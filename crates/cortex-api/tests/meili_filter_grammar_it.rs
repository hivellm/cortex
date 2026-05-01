//! Phase11b §3.5 — grammar-level integration test.
//!
//! `cortex-api::meili_lane::build_meili_filter` emits the exact filter
//! string Meilisearch must parse. This test gates on the live daemon to
//! catch any future grammar regression — Meili is the authority on what
//! filter shapes it accepts, and the unit tests in `meili_lane.rs` only
//! pin the *string we send*, not whether the server accepts it.
//!
//! Opt-in via `CORTEX_MEILI_IT=1` plus `CORTEX_FULLTEXT_MEILI_URL` (and
//! optionally `CORTEX_FULLTEXT_MEILI_KEY` for the master/admin key).
//! When the env var is unset the test becomes a no-op so unsuspecting
//! `cargo test` runs don't try to reach a non-existent server.

use std::env;

use reqwest::Client;
use serde_json::json;

const TEST_INDEX: &str = "cortex-it-meili-filter-grammar";

#[tokio::test]
async fn meili_accepts_path_prefixes_in_filter() {
    if env::var("CORTEX_MEILI_IT").ok().as_deref() != Some("1") {
        eprintln!("CORTEX_MEILI_IT != 1 — skipping live grammar IT");
        return;
    }
    let base = env::var("CORTEX_FULLTEXT_MEILI_URL")
        .expect("CORTEX_FULLTEXT_MEILI_URL must be set when CORTEX_MEILI_IT=1");
    let key = env::var("CORTEX_FULLTEXT_MEILI_KEY").ok();

    let http = Client::builder()
        .build()
        .expect("reqwest client");
    let auth = |req: reqwest::RequestBuilder| match key.as_deref() {
        Some(k) => req.bearer_auth(k),
        None => req,
    };

    // Best-effort tear-down so the test is rerunnable.
    let _ = auth(http.delete(format!("{base}/indexes/{TEST_INDEX}")))
        .send()
        .await;

    auth(
        http.post(format!("{base}/indexes")).json(&json!({
            "uid": TEST_INDEX,
            "primaryKey": "id",
        })),
    )
    .send()
    .await
    .expect("create index");

    auth(
        http.patch(format!("{base}/indexes/{TEST_INDEX}/settings"))
            .json(&json!({
                "filterableAttributes": ["path_prefixes"],
            })),
    )
    .send()
    .await
    .expect("apply settings");

    auth(
        http.post(format!("{base}/indexes/{TEST_INDEX}/documents"))
            .json(&json!([
                {
                    "id": "doc-1",
                    "path_prefixes": [
                        "crates/",
                        "crates/cortex-api/",
                        "crates/cortex-api/src/",
                        "crates/cortex-api/src/meili_lane.rs",
                    ],
                    "body": "fn build_filter() {}",
                }
            ])),
    )
    .send()
    .await
    .expect("upsert doc");

    // Allow Meili a moment to index — the IT is opt-in and rare, so a
    // small fixed sleep beats wiring the task-poll dance into a test
    // that already gated itself on env.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let resp = auth(
        http.post(format!("{base}/indexes/{TEST_INDEX}/search"))
            .json(&json!({
                "q": "",
                "filter": "(path_prefixes IN ['crates/cortex-api/src/'])",
                "limit": 10,
            })),
    )
    .send()
    .await
    .expect("search request");

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Meili rejected the path_prefixes IN filter: status={status} body={body}"
    );
    assert!(
        body.contains("\"doc-1\""),
        "search did not return doc-1 — body={body}"
    );

    // Tear down so a next run starts clean.
    let _ = auth(http.delete(format!("{base}/indexes/{TEST_INDEX}")))
        .send()
        .await;
}
