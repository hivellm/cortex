//! Drift guard — the hook shims under `packages/cortex-claude-plugin/hooks/` must
//! stay byte-identical to the canonical sources under
//! `crates/cortex-adapter-claude-code/hooks/`.
//!
//! Both trees ship the same scripts. Spec 18 + spec 10 want a single
//! source of truth so a fix to one path can't silently leave the
//! other one stale. This test fails the build the moment they
//! diverge — the only way to land a hook change is to update both
//! trees in the same commit.

use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn plugin_hook_shims_match_adapter_canonical_sources() {
    let root = workspace_root();
    let adapter_dir = root.join("crates/cortex-adapter-claude-code/hooks");
    let plugin_dir = root.join("packages/cortex-claude-plugin/hooks");

    let mut canonical: Vec<String> = fs::read_dir(&adapter_dir)
        .expect("adapter hooks dir exists")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.starts_with("cortex-") && (name.ends_with(".sh") || name.ends_with(".ps1"))
        })
        .collect();
    canonical.sort();
    assert!(
        !canonical.is_empty(),
        "no canonical hook shims found under {}",
        adapter_dir.display()
    );

    for name in &canonical {
        let lhs = fs::read(adapter_dir.join(name))
            .unwrap_or_else(|e| panic!("read {} canonical: {e}", name));
        let rhs_path = plugin_dir.join(name);
        let rhs = fs::read(&rhs_path).unwrap_or_else(|e| {
            panic!(
                "plugin shim missing or unreadable at {}: {e}",
                rhs_path.display()
            )
        });
        assert_eq!(
            lhs, rhs,
            "drift between adapter and plugin hook shim: {name}"
        );
    }

    // Reverse direction: plugin must not ship shims the adapter doesn't.
    let plugin_shims: Vec<String> = fs::read_dir(&plugin_dir)
        .expect("plugin hooks dir exists")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.starts_with("cortex-") && (name.ends_with(".sh") || name.ends_with(".ps1"))
        })
        .collect();
    for name in &plugin_shims {
        assert!(
            canonical.contains(name),
            "plugin ships extra shim `{name}` with no canonical counterpart"
        );
    }
}

/// Positive content assert — every `.sh` hook shim MUST contain the
/// `cortex-hook` binary preference branch (phase11x). The byte-equal
/// drift test above catches divergence between plugin + canonical,
/// but if both trees are simultaneously gutted (as happened on
/// commit 14977d4 "tail sweep") the drift stays byte-equal AND
/// broken — Windows hooks silently fall through `nc -U` / `socat`,
/// which are Unix-only, so capture flat-lines without a single
/// CI red.
///
/// The asserts here are intentionally minimal: any rewrite of the
/// lookup ladder must still include these four anchor strings. A
/// future refactor that replaces the `bash` shim with a different
/// dispatch mechanism updates the anchors here in the same commit
/// — the lookup contract is what protects Windows capture, not the
/// specific script shape.
#[test]
fn hook_shims_carry_cortex_hook_bin_preference_branch() {
    let root = workspace_root();
    let dirs = [
        root.join("packages/cortex-claude-plugin/hooks"),
        root.join("crates/cortex-adapter-claude-code/hooks"),
    ];

    // Every shim MUST contain these four anchors so a regression
    // that drops the BIN branch (the 14977d4 incident) fails CI on
    // the first PR push.
    const REQUIRED_ANCHORS: &[&str] = &[
        // 1. Env-override hook for operator-supplied paths.
        "CORTEX_HOOK_BIN",
        // 2. PATH lookup of the native bin.
        "command -v cortex-hook",
        // 3. Fallback to ~/.cortex/ install location (Windows .exe + Unix).
        ".cortex/cortex-hook",
        // 4. `exec` of the resolved BIN so the shim hands off
        //    instead of returning the empty `{}` fallback.
        "exec \"${BIN}\"",
    ];

    for dir in &dirs {
        let mut shims: Vec<String> = fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("cortex-") && name.ends_with(".sh"))
            .collect();
        shims.sort();
        assert!(
            !shims.is_empty(),
            "no .sh shims found under {}",
            dir.display()
        );
        for name in &shims {
            let path = dir.join(name);
            let body = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            for anchor in REQUIRED_ANCHORS {
                assert!(
                    body.contains(anchor),
                    "hook shim {} is missing the `{}` anchor — \
                     the cortex-hook bin preference branch (phase11x) \
                     was dropped. On Windows this collapses capture \
                     to a silent no-op. Restore the BIN lookup ladder \
                     before merging.",
                    path.display(),
                    anchor
                );
            }
        }
    }
}
