## 1. Polyglot the canonical .sh shims
- [ ] 1.1 Add a leading `case "${OSTYPE:-}"` block to every `crates/cortex-adapter-claude-code/hooks/cortex-*.sh` that re-execs the sibling `.ps1` via `pwsh -NoProfile -File "$(dirname "$0")/cortex-<event>.ps1"` on `msys*` / `cygwin*` / `win32*`
- [ ] 1.2 Confirm the existing Unix-socket flow falls through unchanged on `linux-gnu` / `darwin*` (no semantic change to non-Windows paths)

## 2. Fix the .ps1 stdin parsing bug
- [ ] 2.1 In every `crates/cortex-adapter-claude-code/hooks/cortex-*.ps1`, `.Trim()` `$input_text` before embedding it into the JSON frame so a trailing newline no longer reaches the daemon parser
- [ ] 2.2 Confirm a manual `echo '{"prompt":"x"}' | pwsh -File cortex-user-prompt.ps1` no longer triggers `malformed hook frame; replying empty error=EOF` in the daemon log

## 3. Mirror canonical sources into the plugin tree
- [ ] 3.1 Copy every updated `.sh` and `.ps1` from `crates/cortex-adapter-claude-code/hooks/` into `cortex-plugin/hooks/`
- [ ] 3.2 `cargo test -p cortex-mcp-server --test hook_drift` passes (byte-identical mirror)

## 4. Live capture verification
- [ ] 4.1 With `cortex-adapter-claude daemon` listening on the named pipe, run a Claude Code session via `claude --plugin-dir ./cortex-plugin -p "ping"` and confirm at least one well-formed `UserPromptSubmit` frame reaches the daemon (no `malformed hook frame` warnings)
- [ ] 4.2 Inspect daemon stdout / stderr for the absence of parse errors and the presence of the publisher path (HTTP attempt to `cortex-core` is fine — failure there isn't this task's concern)

## 5. Tail (mandatory)
- [ ] 5.1 Update or create documentation covering the implementation — note the `pwsh` Windows prerequisite in `cortex-plugin/README.md`; cross-reference from spec 10's hook-shim section
- [ ] 5.2 Write tests covering the new behavior — add a unit test in `crates/cortex-adapter-claude-code/src/ipc.rs` for `handle_line` that confirms valid JSON surrounded by stray whitespace still parses cleanly (defense-in-depth alongside the .ps1 root-cause fix); drift test already enforces the byte-identical canonical→plugin mirror so the polyglot .sh + Trim()-fixed .ps1 reach the plugin tree
- [ ] 5.3 Run tests and confirm they pass — `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p cortex-mcp-server`, `cargo test -p cortex-adapter-claude-code`, `cargo run -p cortex-mcp-server -- validate ./cortex-plugin` exits 0
