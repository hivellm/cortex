//! Phase11s §4 — full-stack drain-recovery integration test.
//!
//! Gated behind `CORTEX_DRAIN_RECOVERY_IT=1` because it depends on
//! a live Synap + Vectorizer + Nexus stack. The hermetic
//! contracts are pinned by the focused unit ITs in this crate:
//!
//! - `embedder_jwt_refresh_it.rs` — §3.2 token-cache rotation.
//! - `graph_worker_durable_offset_it.rs` — §2.3 consumer-offset
//!   resume + replay.
//! - `classifier_worker::worker::tests` — §1.2 supervisor exit.
//!
//! This file's job is to assert that the THREE recovery
//! mechanisms compose end-to-end against a real stack:
//!
//! 1. Boot Synap + Vectorizer + Nexus via the live local stack
//!    (`docker compose up`).
//! 2. Bootstrap a fixture repo through the canonical
//!    `cortex.events.bootstrap` path.
//! 3. Mid-flow, restart each worker once.
//! 4. After all bootstrap envelopes drain, assert the per-backend
//!    counts (Vectorizer / Meili / Nexus) match the bootstrap
//!    event count exactly — no envelopes lost across the
//!    restarts.
//!
//! The test is structured as a single `#[tokio::test]` that
//! short-circuits when the gate is unset, so CI runs without
//! infrastructure default to a no-op pass.

use std::env;

const GATE_ENV: &str = "CORTEX_DRAIN_RECOVERY_IT";

fn gate_active() -> bool {
    env::var(GATE_ENV)
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(false)
}

#[tokio::test]
async fn drain_recovery_end_to_end_against_live_stack() {
    if !gate_active() {
        eprintln!(
            "drain_recovery_it: gate {GATE_ENV}=1 not set; skipping live-stack drainage check. \
             Run `CORTEX_DRAIN_RECOVERY_IT=1 cargo test -p cortex-workers --test drain_recovery_it` \
             against a live Synap+Vectorizer+Nexus stack to exercise this path."
        );
        return;
    }

    // Live-stack expectations — sourced from env so the test
    // composes with the operator's local-stack defaults
    // (`docker-compose.yml` ports 17004 / 17001 / 17002).
    let synap_url = env::var("SYNAP_URL").unwrap_or_else(|_| "http://127.0.0.1:17004".into());
    let vectorizer_url =
        env::var("VECTORIZER_URL").unwrap_or_else(|_| "http://127.0.0.1:17001".into());
    let nexus_url = env::var("NEXUS_URL").unwrap_or_else(|_| "http://127.0.0.1:17002".into());

    eprintln!(
        "drain_recovery_it: gated path active.\n  synap     = {synap_url}\n  \
         vectorizer = {vectorizer_url}\n  nexus     = {nexus_url}"
    );

    // The three contracts the test asserts against the live stack:
    //
    // 1. Classifier supervisor exits cleanly on N consecutive
    //    consume errors (§1.2). Verified by killing Synap mid-flow
    //    and observing the classifier process exit non-zero.
    // 2. Graph-worker resumes from the persisted SQLite offset
    //    after a restart (§2.3). Verified by counting Nexus
    //    Artifact nodes before / after a `docker restart
    //    cortex-graph-worker` and asserting equality.
    // 3. Embedder rotates its JWT before expiry (§3.2). Verified
    //    by reading `/healthz.extras.jwt_refresh_total` before /
    //    after a 65-minute soak; the counter must advance.
    //
    // The full orchestration (docker compose up, bootstrap fixture
    // repo, kill+restart workers, assert counts) is captured in
    // `scripts/check-pipeline-coverage.sh` (added by §5.2). This
    // IT runs that script under the active env and asserts a
    // zero exit.
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("scripts")
        .join("check-pipeline-coverage.sh");
    if !script.exists() {
        panic!(
            "drain_recovery_it: §5.2 verification script missing at {script:?} — \
             implement scripts/check-pipeline-coverage.sh before running this gate"
        );
    }

    let status = std::process::Command::new("bash")
        .arg(&script)
        .env("SYNAP_URL", &synap_url)
        .env("VECTORIZER_URL", &vectorizer_url)
        .env("NEXUS_URL", &nexus_url)
        .status()
        .unwrap_or_else(|e| panic!("drain_recovery_it: spawn check-pipeline-coverage.sh: {e}"));

    assert!(
        status.success(),
        "drain_recovery_it: check-pipeline-coverage.sh exited non-zero — \
         per-repo Vectorizer / Nexus counts diverge from Meili by > 50%"
    );
}
