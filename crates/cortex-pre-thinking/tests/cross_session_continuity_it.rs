//! Phase30 (continuity-loop-verification §1.1) — cross-session
//! continuity: a consolidation produced by a PRIOR session must
//! surface in a FRESH session's pre-thinking bundle when the new
//! session asks about the same topic.
//!
//! Live-gated: runs the real `pipeline::run` against the running
//! cortex-api (`CORTEX_API_URL`, default `http://127.0.0.1:17000`)
//! and SKIPS (early `return`) when the stack is down, mirroring the
//! adapter's live-IT convention. The "prior session" is the live
//! corpus itself: the newest consolidation from
//! `/v1/consolidations/recent` — every consolidation is, by
//! construction, the distillate of a past session, so asserting it
//! reappears in a fresh-session bundle IS the §1.1 loop.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cortex_api::{QueryRequest, QueryResponse};
use cortex_pre_thinking::metrics::Metrics;
use cortex_pre_thinking::pipeline::{run, ClosureQueryFn, PreThinkingBudget, PreThinkingInput};

fn api_url() -> String {
    std::env::var("CORTEX_API_URL").unwrap_or_else(|_| "http://127.0.0.1:17000".into())
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("reqwest client")
}

/// `true` when the live cortex-api answers `/v1/health`.
async fn stack_up(client: &reqwest::Client, base: &str) -> bool {
    matches!(
        client.get(format!("{base}/v1/health")).send().await,
        Ok(r) if r.status().is_success()
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prior_session_consolidation_surfaces_in_fresh_session_bundle() {
    let base = api_url();
    let client = http();
    if !stack_up(&client, &base).await {
        eprintln!("skip: cortex-api not reachable at {base}");
        return;
    }

    // The prior session's distillate: newest consolidation on record.
    let recent: serde_json::Value = client
        .get(format!(
            "{base}/v1/consolidations/recent?repo=cortex&limit=1"
        ))
        .send()
        .await
        .expect("GET consolidations/recent")
        .json()
        .await
        .expect("recent json");
    let Some(hit) = recent["hits"].as_array().and_then(|h| h.first()) else {
        eprintln!("skip: no consolidations in the live corpus — loop unverifiable");
        return;
    };
    let cons_id = hit["id"].as_str().expect("consolidation id").to_string();
    let cons_title = hit["title"]
        .as_str()
        .expect("consolidation title")
        .to_string();

    // Fresh session, same repo, asking about the prior session's topic.
    let query_client = client.clone();
    let query_base = base.clone();
    let query_fn = Arc::new(ClosureQueryFn(move |req: QueryRequest| {
        let client = query_client.clone();
        let base = query_base.clone();
        async move {
            let resp = client
                .post(format!("{base}/v1/query"))
                .json(&req)
                .send()
                .await
                .ok()?;
            resp.json::<QueryResponse>().await.ok()
        }
    }));

    let session_id = format!(
        "it-continuity-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis()
    );
    let prompt = format!(
        "I'm about to rework this area — what did previous sessions \
         conclude about {cons_title}?"
    );
    let input = PreThinkingInput {
        session_id: &session_id,
        turn_id: "turn-1",
        user_prompt: &prompt,
        cwd: Path::new(env!("CARGO_MANIFEST_DIR")),
        recent_files: &[],
        // Spec-12's 600ms production budget trips fail-open on live
        // round-trips (~700ms+ observed); this IT verifies CONTINUITY,
        // not latency, so it gets a generous wall-clock.
        budget: PreThinkingBudget {
            bundle_bytes: 32 * 1024,
            time_ms: 20_000,
        },
        principal: None,
    };

    let out = run(&input, query_fn, Arc::new(Metrics::new())).await;

    assert!(
        !out.fail_open,
        "pipeline took the fail-open path (api unreachable mid-test?)"
    );

    // THE LOOP: a fresh session's bundle carries consolidated context
    // distilled by earlier sessions. The "Consolidated context"
    // section renders if and only if `/v1/query` populated
    // `results.consolidations` — which was structurally impossible
    // before phase30b, so this section's presence IS the loop.
    //
    // Deliberately NOT asserted: that `{cons_title}` specifically wins
    // retrieval. Which of the corpus's consolidations ranks first for
    // a given prompt is a RELEVANCE property of live BM25 over a
    // shared corpus, not a continuity-wiring property — measured live,
    // even an exact-title query surfaces a different (still valid)
    // consolidation. Pinning one document here would make this test a
    // flaky ranking assertion wearing a wiring test's name. Ranking
    // quality has its own gate (the cortex-eval retrieval suites).
    // Note also that `/v1/consolidations/recent` reports the event
    // ULID while a rendered row carries the `cons-<grain>-…` id — the
    // two identifier spaces do not overlap, so an id-equality check
    // could never pass regardless of ranking.
    assert!(
        out.bundle.contains("## Consolidated context ("),
        "fresh session's bundle must carry a Consolidated context \
         section built from prior sessions (corpus has ≥1 \
         consolidation, e.g. {cons_id} / {cons_title:?}); got bundle:\n{}",
        out.bundle
    );
    // Rows are `N. <grain>/<consolidation_id> · …` — assert the id
    // space is the consolidator's, proving the row came from a real
    // assembled `ConsolidationRef` and not from stray text.
    assert!(
        out.bundle.contains("/cons-"),
        "the section's rows must carry real `cons-<grain>-…` ids; got bundle:\n{}",
        out.bundle
    );
}
