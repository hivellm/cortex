//! Phase13e §4.3 — IT confirms `cortex_config::audit()` against the
//! live workspace reports zero ad-hoc CORTEX_* call sites outside
//! the audit's exclusion list (cortex-config, cortex-build, tests).
//!
//! This is the runtime counterpart to the §3.6 CI grep gate. When
//! both surfaces report zero, ADR-016 §3 is fully realised.
//!
//! Runs on a dedicated thread with an 8 MiB stack because walking
//! the workspace's `crates/` tree on Windows (1 MiB default main
//! stack) recursively in regex's hot path overflows the default.

use std::path::PathBuf;

#[test]
fn workspace_audit_reports_zero_ad_hoc_cortex_env_reads() {
    let crates_root = workspace_crates_root();
    if !crates_root.is_dir() {
        // Test only runs from a workspace checkout. Skip silently
        // when the layout is unfamiliar (e.g. a sdist consumer
        // running our test binary out of context).
        eprintln!(
            "workspace_audit_it: skipping — no crates/ dir at {}",
            crates_root.display()
        );
        return;
    }
    let report = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || cortex_config::audit(&crates_root))
        .expect("spawn 8 MiB stack thread")
        .join()
        .expect("audit thread joined");
    assert!(
        report.is_empty(),
        "ADR-016 §3.6 — ad-hoc CORTEX_* reads remain outside the typed Config: {report:#?}"
    );
}

fn workspace_crates_root() -> PathBuf {
    // Walk up from CARGO_MANIFEST_DIR looking for a sibling
    // `crates/` directory. Mirrors the resolution
    // `cortex-ops doctor-config-audit` performs.
    let mut cur = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = cur.join("crates");
        if candidate.is_dir() {
            return candidate;
        }
        if !cur.pop() {
            return PathBuf::from("crates");
        }
    }
}
