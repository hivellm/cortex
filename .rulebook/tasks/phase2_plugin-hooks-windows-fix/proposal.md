# Proposal: phase2_plugin-hooks-windows-fix

## Why

The spec-18 plugin merge (`phase2_plugin-hooks-merge`) shipped
`cortex-plugin/hooks/hooks.json` invoking `bash "${CLAUDE_PLUGIN_ROOT}/hooks/cortex-<event>.sh"`.
On Linux/macOS that works: the `.sh` shim talks to the
`~/.cortex/adapter-claude.sock` Unix socket via `nc -U` / `socat`. On
Windows it doesn't — Git Bash doesn't ship `nc`, and even if it did,
the daemon binds a Windows named pipe (`\\.\pipe\cortex-adapter-claude`),
not a Unix socket.

End-to-end smoke test confirmed two distinct gaps:

1. The plugin's hook entry path picks the `.sh` shim regardless of
   platform — wrong artifact on Windows.
2. The canonical `.ps1` shim is reachable, but emits a malformed
   frame (trailing newline from stdin embedded into the JSON object).
   Daemon log: `malformed hook frame; replying empty error=EOF while
   parsing an object at line 1 column 120`.

Capture is silently no-op on Windows today. Both gaps must close so
the spec-18 plugin's headline promise ("one install, both surfaces")
holds on every platform we care about.

## What Changes

- The canonical `.sh` shims under
  `crates/cortex-adapter-claude-code/hooks/` switch to a polyglot
  shape: a leading `case "${OSTYPE:-}" in msys*|cygwin*|win32*)` block
  re-execs the sibling `.ps1` via `pwsh -NoProfile -File`. On Linux /
  macOS the case falls through and the existing Unix-socket logic
  runs unchanged. The plugin's `hooks/hooks.json` keeps invoking
  `bash <event>.sh` — no per-platform `command` strings, no new
  dispatcher, no validator changes.
- The canonical `.ps1` shims `.Trim()` `$input_text` before embedding
  it as the `payload` JSON value, so a stdin with a trailing newline
  no longer breaks the wire frame. The daemon parser stops misaligning
  on the embedded newline.
- The plugin tree mirrors the updated canonical sources via the same
  byte-identical drift contract (`cargo test -p cortex-mcp-server
  --test hook_drift`).
- README adds a short Windows-prereq line: `pwsh` 7+ on `PATH` (Claude
  Code already requires PowerShell on Windows for its own shims).

## Impact

- Affected specs: spec 10 (canonical shim catalogue updated), spec 18
  (no schema change — hooks.json shape is unaffected). No spec status
  flips.
- Affected code: 14 shim files under
  `crates/cortex-adapter-claude-code/hooks/` plus their plugin mirrors.
  No Rust changes (validator, installer, drift test all stay).
- Breaking change: NO — Linux / macOS behaviour is byte-identical.
- User benefit: capture works out of the box on Windows after
  `claude plugin install cortex@hivellm-cortex` plus
  `cortex-adapter-claude daemon`. No two-codepath maintenance burden.
