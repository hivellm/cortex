# Proposal: phase2_plugin-hooks-merge

## Why

Spec 10 (capture) and spec 18 (plugin) currently install through two
independent code paths: `cortex-adapter-claude install` writes hook
shims into `~/.claude/hooks/` and patches the global `settings.json`,
while `claude plugin install cortex@hivellm-cortex` only registers the
MCP server tools. The split means a fresh `claude plugin install`
produces silence — the model can pull from Cortex on demand, but no
session events flow in, because the hook adapter wasn't installed.

The Claude Code plugin format already supports a `hooks/hooks.json`
descriptor that registers hooks at plugin-install time. Folding the
spec-10 shim catalogue into the plugin tree fixes the mismatch:
installing the plugin gets you both surfaces (capture + tools) and the
standalone `cortex-adapter-claude install` becomes optional (kept for
non-plugin contexts and CI).

## What Changes

- New `cortex-plugin/hooks/` directory shipping every shim from
  `crates/cortex-adapter-claude-code/hooks/` (sh + ps1) plus a
  `hooks/hooks.json` descriptor that maps `SessionStart`,
  `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`,
  `SubagentStop`, `Notification` to `${CLAUDE_PLUGIN_ROOT}/hooks/cortex-*.sh`.
- The shim catalogue stays single-sourced: `cortex-adapter-claude-code`
  keeps the canonical scripts under `crates/cortex-adapter-claude-code/hooks/`,
  and the plugin tree mirrors them via a CI-checked copy so the two
  never drift. The mirroring lines up with the same `HOOK_SHIMS` array
  the spec-10 installer already uses.
- `cortex-mcp-server validate` learns to lint the new `hooks/`
  directory — refuses to ship if `hooks.json` references a missing
  script or if a script in `hooks/` isn't referenced from `hooks.json`.
- Spec 18 acceptance criteria add: hooks present and lint-clean,
  install drill verifies hook entries register, capture round-trip
  test confirms a `UserPromptSubmit` event makes it through to
  `cortex-api` after a `claude plugin install`.
- Spec 10's install path stays for non-plugin users but flips status
  to "optional / superseded by spec 18 when plugin is in use".
  `cortex-adapter-claude install` keeps working unchanged so existing
  installations don't break, and a new `--no-hooks` flag lets plugin
  users opt the hook-shim step out (the adapter daemon still runs and
  publishes when invoked).
- README + `cortex-plugin/README.md` document the new single-install
  path and add a migration note for users who already ran the spec-10
  installer.

## Impact

- Affected specs: spec 18 (acceptance criteria + decisions update),
  spec 10 (install path notes flip to optional). Spec 11 / spec 12
  unchanged.
- Affected code: `cortex-plugin/hooks/` (new), validator
  (`crates/cortex-mcp-server/src/validate.rs`), shim mirror lint,
  spec-10 installer (`--no-hooks` flag).
- Breaking change: NO — additive. Existing spec-10 installs keep
  working; plugin install now also wires hooks; running both produces
  duplicate hook firing (mitigated by docs + `--no-hooks` flag).
- User benefit: one command (`claude plugin install
  cortex@hivellm-cortex` once `cortex-mcp-server` and
  `cortex-adapter-claude-code` binaries are on `PATH`) gets the full
  Cortex experience — capture + retrieval + pre-thinking — instead of
  the previous two-step drill.
