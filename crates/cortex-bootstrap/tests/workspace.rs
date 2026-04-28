//! Integration tests for the workspace orchestrator (phase4b §3+§5).
//!
//! Covers the load + preflight loop end-to-end against synthetic
//! git repos on disk. The CLI binary itself is not exercised here —
//! the orchestration logic between workspace config and
//! `run_repo` is what the tests pin.

use std::fs;
use std::path::Path;

use cortex_bootstrap::{
    load_workspace, preflight_workspace, WorkspaceConfig, WorkspaceError, WorkspaceRepo,
};

fn write_synthetic_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(
        root.join("cortex.toml"),
        "[cortex]\nid = \"Synthetic\"\n",
    )
    .unwrap();
}

#[test]
fn preflight_accepts_a_two_repo_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let r1 = tmp.path().join("R1");
    let r2 = tmp.path().join("R2");
    write_synthetic_repo(&r1);
    write_synthetic_repo(&r2);

    let cfg = WorkspaceConfig {
        repos: vec![
            WorkspaceRepo {
                id: "R1".into(),
                path: r1,
                config: None,
            },
            WorkspaceRepo {
                id: "R2".into(),
                path: r2,
                config: None,
            },
        ],
    };
    preflight_workspace(&cfg).expect("preflight should pass");
}

#[test]
fn load_workspace_then_preflight_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let r1 = tmp.path().join("R1");
    let r2 = tmp.path().join("R2");
    write_synthetic_repo(&r1);
    write_synthetic_repo(&r2);

    let ws_path = tmp.path().join("ws.toml");
    let body = format!(
        "[[repo]]\nid = \"R1\"\npath = \"{}\"\n\n[[repo]]\nid = \"R2\"\npath = \"{}\"\n",
        r1.display().to_string().replace('\\', "/"),
        r2.display().to_string().replace('\\', "/"),
    );
    fs::write(&ws_path, body).unwrap();

    let loaded = load_workspace(&ws_path).expect("load");
    assert_eq!(loaded.repos.len(), 2);
    preflight_workspace(&loaded).expect("preflight after load");
}

#[test]
fn preflight_aborts_when_one_path_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let good = tmp.path().join("Good");
    write_synthetic_repo(&good);
    let absent = tmp.path().join("Absent");

    let cfg = WorkspaceConfig {
        repos: vec![
            WorkspaceRepo {
                id: "Good".into(),
                path: good,
                config: None,
            },
            WorkspaceRepo {
                id: "Absent".into(),
                path: absent,
                config: None,
            },
        ],
    };
    let err = preflight_workspace(&cfg).expect_err("preflight should fail");
    let WorkspaceError::Preflight(lines) = err else {
        panic!("expected Preflight variant");
    };
    assert!(
        lines.iter().any(|l| l.contains("Absent") && l.contains("does not exist")),
        "expected absent-path entry, got {lines:?}",
    );
    // Good entry must NOT have produced a failure line.
    assert!(
        !lines.iter().any(|l| l.contains("`Good`")),
        "Good entry should not trigger any failure, got {lines:?}",
    );
}
