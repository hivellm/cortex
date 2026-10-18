//! Separate contract guards for standalone plugin wiring and retained legacy adapter shims.
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
fn standalone_plugins_register_only_local_supported_events() {
    let root = workspace_root();
    let expected = std::collections::BTreeSet::from([
        "SessionStart",
        "UserPromptSubmit",
        "PostToolUse",
        "Stop",
        "SubagentStop",
    ]);
    for plugin in ["cortex-claude-plugin", "cortex-codex-plugin"] {
        let base = root.join("packages").join(plugin);
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(base.join("hooks/hooks.json")).unwrap()).unwrap();
        let actual: std::collections::BTreeSet<&str> = value["hooks"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(expected, actual);
        let mcp = if plugin.contains("codex") {
            "mcp.json"
        } else {
            ".mcp.json"
        };
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(base.join(mcp)).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["cortex"]["command"], "cortex");
        assert_eq!(
            config["mcpServers"]["cortex"]["args"],
            serde_json::json!(["mcp"])
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
/// lookup ladder must still include the lookup anchors and quoted exec. A
/// future refactor that replaces the `bash` shim with a different
/// dispatch mechanism updates the anchors here in the same commit
/// — the lookup contract is what protects Windows capture, not the
/// specific script shape.
#[test]
fn hook_shims_carry_cortex_hook_bin_preference_branch() {
    let root = workspace_root();
    // Legacy adapter remains independently supported; standalone plugins no longer share its runtime.
    let dirs = [root.join("crates/cortex-adapter-claude-code/hooks")];

    // Every shim MUST contain these lookup anchors and a quoted native exec.
    // A regression dropping the BIN branch (the 14977d4 incident) must fail CI
    // on the first PR push.
    const REQUIRED_ANCHORS: &[&str] = &[
        // 1. Env-override hook for operator-supplied paths.
        "CORTEX_HOOK_BIN",
        // 2. PATH lookup of the native bin.
        "command -v cortex-hook",
        // 3. Fallback to ~/.cortex/ install location (Windows .exe + Unix).
        ".cortex/cortex-hook",
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
            assert!(
                body.contains("exec \"${BIN}\"") || body.contains("exec \"$BIN\""),
                "hook shim {} must exec the quoted native BIN",
                path.display()
            );
            if matches!(
                name.as_str(),
                "cortex-session-start.sh" | "cortex-user-prompt.sh"
            ) {
                assert!(body.contains("exec \"$BIN\" \"$EVENT\""));
                assert!(body.contains("SYNCHRONOUS=1"));
                assert!(body
                    .lines()
                    .filter(|line| !line.trim_start().starts_with('#'))
                    .all(|line| !line.contains("--fire-forget")));
            }
        }
    }
}
