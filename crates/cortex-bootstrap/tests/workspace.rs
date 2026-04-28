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

/// Phase4g — guard the source-controlled
/// `bootstrap.workspace.toml.example` template against drift.
///
/// The template carries a literal `${HIVE_ROOT}` token the operator
/// search-and-replaces post-checkout, so `preflight_workspace` cannot
/// run here (those token-bearing paths do not resolve). Loading via
/// `load_workspace` is enough to catch the failure modes that
/// actually fire on the operator path: TOML parse errors, missing
/// required fields (`id` / `path`), and accidental drift in the entry
/// count. The 17 expected entries match the HiveLLM repo list
/// documented in `docs/operations/bootstrap-workspace.md`.
#[test]
fn bootstrap_workspace_example_loads() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let template = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crate dir has a workspace root")
        .join("bootstrap.workspace.toml.example");
    assert!(
        template.exists(),
        "template missing at {}",
        template.display()
    );

    let cfg = load_workspace(&template).expect("template parses cleanly");
    assert_eq!(
        cfg.repos.len(),
        17,
        "expected 17 entries in the workspace template, got {}",
        cfg.repos.len()
    );

    // Cortex must be entry #0 — the operator usually drives bootstrap
    // from the Cortex checkout itself, and the orchestrator iterates
    // entries in declaration order.
    assert_eq!(cfg.repos[0].id, "Cortex");

    let mut seen = std::collections::BTreeSet::new();
    for repo in &cfg.repos {
        assert!(
            !repo.id.trim().is_empty(),
            "entry has empty id: {repo:?}",
        );
        assert!(
            seen.insert(repo.id.clone()),
            "duplicate id `{}` in template",
            repo.id,
        );
        // Path must reference the literal `${HIVE_ROOT}` token so the
        // operator's search-and-replace target is unambiguous.
        let path = repo.path.to_string_lossy();
        assert!(
            path.contains("${HIVE_ROOT}"),
            "entry `{}` is missing the ${{HIVE_ROOT}} token: {}",
            repo.id,
            path,
        );
    }

    // `Cortex` / `Vectorizer` / `Nexus` / `Synap` / `Rulebook` are the
    // load-bearing repos the rest of the stack depends on. Pin them
    // explicitly so a future template edit cannot drop them.
    for required in ["Cortex", "Vectorizer", "Nexus", "Synap", "Rulebook"] {
        assert!(
            seen.contains(required),
            "template is missing required repo `{required}`",
        );
    }
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
