//! Phase11k §3.3 — bootstrap law extraction integration test.
//!
//! Fixture ships an AGENTS.override.md carrying two LAW-CORTEX-*
//! declarations. The bootstrap walker must classify the file as Law
//! (because `[cortex.laws].promote_patterns` lists it) and the
//! emitter must split the body by `## LAW-...` heading using the
//! `[cortex.laws].extract_pattern`, producing one `law.imported`
//! envelope per match — not one envelope for the whole file.

use std::fs;
use std::sync::Arc;

use cortex_cli::bootstrap::{
    run_repo, Checkpoint, CortexSection, MemoryPublisher, Metrics, Publisher, RunnerConfig,
};
use serde_json::Value;
use tempfile::TempDir;

fn fixture_with_two_laws() -> TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let body = "\
# Project-Specific Overrides

## LAW-CORTEX-001 — Strict task-sequence execution

**Trigger:** any time work is governed by a `tasks.md` checklist.

**Rule:** Execute every checklist top-to-bottom in the EXACT order listed.

## LAW-CORTEX-002 — Reserved

(Keeping numbering reserved so future laws extend cleanly.)
";
    fs::write(root.join("AGENTS.override.md"), body).unwrap();
    tmp
}

fn fixture_config() -> CortexSection {
    let body = r#"
[cortex]
id = "Fixture"

[cortex.laws]
promote_patterns = ["AGENTS.override.md", "AGENTS.md"]
extract_pattern = "^LAW-[A-Z0-9-]+$"

[cortex.git]
include_commits = false
"#;
    let parsed: cortex_cli::bootstrap::CortexToml = toml::from_str(body).unwrap();
    parsed.cortex
}

#[tokio::test]
async fn agents_override_emits_one_law_envelope_per_law_cortex_heading() {
    let repo = fixture_with_two_laws();
    let cfg = fixture_config();
    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let metrics = Arc::new(Metrics::new());
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };
    run_repo(
        repo.path(),
        &runner_cfg,
        &cfg,
        pub_dyn,
        metrics,
        &mut checkpoint,
        None,
        None,
    )
    .await
    .expect("run_repo");

    let laws = publisher.by_kind("law.imported");
    assert_eq!(
        laws.len(),
        2,
        "AGENTS.override.md must fan out into 2 law.imported envelopes (got {})",
        laws.len(),
    );

    let mut law_ids: Vec<String> = laws
        .iter()
        .filter_map(|evt| {
            evt.redacted_payload
                .get("law_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    law_ids.sort();
    assert_eq!(
        law_ids,
        vec!["LAW-CORTEX-001".to_string(), "LAW-CORTEX-002".to_string()],
        "law ids must come from the matching `## LAW-CORTEX-NNN` headings",
    );

    // Pin the per-section body shape: each envelope's body must
    // include the heading that produced it (so dashboard renderers
    // surface the right slice) and MUST NOT include the OTHER law's
    // section (or the extraction merged the file into a single
    // payload — the regression we're guarding against).
    let first = laws
        .iter()
        .find(|e| {
            e.redacted_payload.get("law_id").and_then(Value::as_str) == Some("LAW-CORTEX-001")
        })
        .expect("LAW-CORTEX-001 envelope present");
    let body = first
        .redacted_payload
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        body.contains("LAW-CORTEX-001"),
        "first envelope must carry its heading; body=`{body}`",
    );
    assert!(
        !body.contains("LAW-CORTEX-002"),
        "first envelope must not bleed into the next law's section; body=`{body}`",
    );
}

#[tokio::test]
async fn law_files_without_matching_headings_fall_back_to_single_envelope() {
    // Phase11k §3.2 — when extract_pattern is set but the body has
    // no matching `## LAW-...` heading, the emitter falls back to
    // the single-law-per-file shape so legacy `.claude/rules/*.md`
    // and similar files keep working.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join(".claude/rules")).unwrap();
    fs::write(
        root.join(".claude/rules/no-shortcuts.md"),
        "# No shortcuts\n\nNever use stubs or placeholders.\n",
    )
    .unwrap();

    let cfg_body = r#"
[cortex]
id = "Fixture"

[cortex.laws]
promote_patterns = [".claude/rules/*.md"]
extract_pattern = "^LAW-[A-Z0-9-]+$"

[cortex.git]
include_commits = false
"#;
    let parsed: cortex_cli::bootstrap::CortexToml = toml::from_str(cfg_body).unwrap();
    let cfg = parsed.cortex;

    let publisher = Arc::new(MemoryPublisher::new());
    let pub_dyn: Arc<dyn Publisher> = publisher.clone();
    let metrics = Arc::new(Metrics::new());
    let mut checkpoint = Checkpoint::new("now".into());
    let runner_cfg = RunnerConfig {
        repo_id: "Fixture".into(),
        stream: "cortex.events.bootstrap".into(),
        since: None,
        dry_run: false,
        kind_filter: Vec::new(),
    };
    run_repo(
        root,
        &runner_cfg,
        &cfg,
        pub_dyn,
        metrics,
        &mut checkpoint,
        None,
        None,
    )
    .await
    .expect("run_repo");

    let laws = publisher.by_kind("law.imported");
    assert_eq!(
        laws.len(),
        1,
        "no `## LAW-` heading → single-law-per-file fallback",
    );
}
