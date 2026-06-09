## 1. Emit array/matcher hook format
- [x] 1.1 Rewrite `install.rs::patch_settings` / `build_hook_entry` to write the array/matcher form per event (`matcher: "*"` for PreToolUse/PostToolUse; bare `{hooks:[...]}` group otherwise).
- [x] 1.2 In-place migration: when an existing legacy-object cortex entry is found under an event, replace it with the array form (a plain `install` heals an old `~/.claude/settings.json`).
- [x] 1.3 Keep the idempotent scan: non-cortex hooks (rulebook, user) survive install + uninstall untouched.

## 2. Uninstall + plugin parity
- [x] 2.1 Update `uninstall` to locate + strip the array-form cortex stanza (and still clean a legacy-object one).
- [x] 2.2 Mirror the array form in the spec-18 plugin `hooks/` if it ships the same shape. (N/A — plugin `hooks/hooks.json` already used array/matcher form; no change needed.)

## 3. Tail (mandatory)
- [x] 3.1 Update adapter spec (docs/specs/10-*) + `CHANGELOG.md` with the format migration.
- [x] 3.2 Tests: install into a temp HOME asserts the array shape + idempotent re-install + legacy-object migration + non-cortex-hook survival.
- [x] 3.3 `cargo check --workspace && cargo clippy -p cortex-adapter-claude-code -- -D warnings && cargo test -p cortex-adapter-claude-code` clean.
## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation.
- [x] 99.2 Write tests covering the new behavior.
- [x] 99.3 Run tests and confirm they pass.
