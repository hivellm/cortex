//! Drift guard — the hook shims under `cortex-plugin/hooks/` must
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
    let plugin_dir = root.join("cortex-plugin/hooks");

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
