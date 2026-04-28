//! Phase4j — CI gate guard tests.
//!
//! Three lightweight assertions against the source-controlled
//! Makefile target and `.github/workflows/doctor.yml` workflow:
//!
//! 1. The Makefile carries a `doctor-consistency` phony target that
//!    shells the canonical `cargo run -p cortex-ops -- doctor-consistency`
//!    command — drift here would silently turn the make target into
//!    a no-op.
//! 2. The workflow runs the same cargo command in JSON mode and
//!    uploads the report artifact — a typo in the artifact name or
//!    the `--json` flag would silently lose the postmortem trail.
//! 3. The workflow brings up `docker compose` and runs
//!    `cortex-bootstrap --workspace` — without these the doctor sees
//!    an empty archive and the gate becomes vacuous.
//!
//! Equivalent to "did anyone forget to update the Makefile when the
//! cargo path moved" — the cheapest possible CI-gate-of-the-CI-gate.

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate dir has a workspace root")
        .to_path_buf()
}

#[test]
fn makefile_has_doctor_consistency_target_with_canonical_cargo_command() {
    let makefile = workspace_root().join("Makefile");
    let body = std::fs::read_to_string(&makefile)
        .unwrap_or_else(|e| panic!("read {}: {e}", makefile.display()));
    assert!(
        body.contains("doctor-consistency:"),
        "Makefile is missing the doctor-consistency target",
    );
    // The .PHONY declaration must list the target so `make` re-runs
    // it even when a stale `doctor-consistency` file exists.
    assert!(
        body.contains(".PHONY:") && body.contains("doctor-consistency"),
        "doctor-consistency must be declared .PHONY",
    );
    // The recipe must shell the canonical cargo command. Match
    // loose on whitespace so the test doesn't break on tab/space drift.
    let recipe_ok = body
        .lines()
        .any(|l| l.contains("cargo run") && l.contains("cortex-ops") && l.contains("doctor-consistency"));
    assert!(
        recipe_ok,
        "doctor-consistency recipe must shell `cargo run -p cortex-ops -- doctor-consistency`",
    );
}

#[test]
fn doctor_workflow_emits_json_report_and_uploads_artifact() {
    let workflow = workspace_root().join(".github/workflows/doctor.yml");
    let body = std::fs::read_to_string(&workflow)
        .unwrap_or_else(|e| panic!("read {}: {e}", workflow.display()));
    // Cargo command runs the doctor in JSON mode — the gate uploads
    // the structured report, not the markdown table.
    assert!(
        body.contains("doctor-consistency --json"),
        "workflow must run the doctor in --json mode for the artifact",
    );
    // Artifact upload step ships the report so a failed gate has a
    // postmortem trail. The artifact name is referenced from the
    // spec doc, so it is part of the contract.
    assert!(
        body.contains("doctor-consistency-report"),
        "workflow must upload the report under name `doctor-consistency-report`",
    );
    assert!(
        body.contains("upload-artifact"),
        "workflow must use actions/upload-artifact",
    );
}

#[test]
fn doctor_workflow_brings_up_compose_and_seeds_bootstrap() {
    let workflow = workspace_root().join(".github/workflows/doctor.yml");
    let body = std::fs::read_to_string(&workflow)
        .unwrap_or_else(|e| panic!("read {}: {e}", workflow.display()));
    // Without docker compose the doctor has nothing to probe — the
    // gate must bring the stack up before running the cargo command.
    assert!(
        body.contains("docker compose up"),
        "workflow must bring up the docker compose stack",
    );
    // Bootstrap is what populates the archive + Meili / Vectorizer /
    // Nexus that the doctor compares; without it the doctor would
    // report "archive empty" and the gate would be vacuous.
    assert!(
        body.contains("cortex-bootstrap"),
        "workflow must run cortex-bootstrap to seed the stack before probing",
    );
    assert!(
        body.contains("--workspace"),
        "workflow must drive bootstrap via the workspace TOML, not positional args",
    );
}
