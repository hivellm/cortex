## 1. Mirror hook shims into the plugin tree
- [x] 1.1 Create `cortex-plugin/hooks/` and copy every `.sh` + `.ps1` from `crates/cortex-adapter-claude-code/hooks/` into it
- [x] 1.2 Write `cortex-plugin/hooks/hooks.json` mapping each Claude Code event (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStop`, `Notification`) to `${CLAUDE_PLUGIN_ROOT}/hooks/cortex-<event>.sh` via `bash`
- [x] 1.3 Drift-check: a unit test asserts the `cortex-plugin/hooks/cortex-*.sh` and `.ps1` files are byte-identical to the canonical sources under `crates/cortex-adapter-claude-code/hooks/`

## 2. Validator coverage
- [x] 2.1 Extend `cortex-mcp-server validate` to load `cortex-plugin/hooks/hooks.json`, parse the shape, and assert every referenced script exists relative to the plugin root
- [x] 2.2 Validator also fails when a script lives under `hooks/` but is not referenced from `hooks.json` (no orphan shims)
- [x] 2.3 Unit tests cover: clean tree passes; missing `hooks.json` fails; orphan script fails; broken `${CLAUDE_PLUGIN_ROOT}` reference fails

## 3. Spec-10 installer flag
- [x] 3.1 Add `--no-hooks` to `cortex-adapter-claude install` — when set, the installer omits the hook-shim write + the `settings.json` patch but keeps the daemon socket + adapter binary install
- [x] 3.2 Update `InstallReport` to surface the omission explicitly (`hooks_omitted: true`) and adjust the existing tests
- [x] 3.3 README adds the recommended path: install via plugin marketplace; run `cortex-adapter-claude install --no-hooks` only if you also want the standalone daemon

## 4. Specs
- [x] 4.1 Update `docs/specs/18-claude-code-plugin.md` — add `hooks/` to the directory layout, append the hooks-related acceptance criteria, add a Decision noting that hooks ship inside the plugin (single-install path)
- [x] 4.2 Update `docs/specs/10-claude-code-adapter.md` — note that the hook-shim install path is optional when spec 18 is in use; reference the `--no-hooks` flag

## 5. Tail (mandatory)
- [x] 5.1 Update or create documentation covering the implementation — flip `cortex-plugin/README.md` and the spec changes above; add a "Migrating from spec-10 standalone install" subsection with the uninstall-then-reinstall drill
- [x] 5.2 Write tests covering the new behavior — validator unit tests, drift-check unit test, install integration test confirming `--no-hooks` omits the shim write but leaves settings.json byte-identical to pre-install state
- [x] 5.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p cortex-mcp-server`, `cargo test -p cortex-adapter-claude-code`, `cargo run -p cortex-mcp-server -- validate ./cortex-plugin` exits 0, and a local `claude plugin install` round-trip shows the hooks registered (`claude plugin list` + `~/.claude/settings.json` reflect the cortex hook entries)
