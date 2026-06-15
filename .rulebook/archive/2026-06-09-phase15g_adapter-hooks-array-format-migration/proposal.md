# Proposal: phase15g_adapter-hooks-array-format-migration

Source: 2026-06-09 live incident — entire ingestion pipeline silent since 2026-06-06.

## Why

`cortex-adapter-claude-code`'s installer writes Claude Code hook entries to
`~/.claude/settings.json` in the **legacy object form**:

```json
"hooks": { "PreToolUse": { "type": "command", "command": "cortex-hook PreToolUse" } }
```

Current Claude Code only honours the **array / matcher form**:

```json
"hooks": { "PreToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": "cortex-hook PreToolUse" } ] } ] }
```

After a Claude Code update (~2026-06-06) the old-format hooks silently stopped
firing. The adapter daemon stopped receiving frames (`frames_received` froze at
the 06-06 timestamp despite live sessions), so ingestion → Synap → all four
workers (classifier / embedder / fulltext / graph) starved — the dashboard went
fully DEGRADED with a ~51h freshness gap, even though every component was
individually healthy. A manual `~/.claude/settings.json` hook conversion +
`cortex-hook` IPC test on 2026-06-09 restored full end-to-end flow (classifier
`jobs_processed_total` advanced immediately), confirming the format was the
only break. That manual fix will regress the next time `cortex-adapter-claude
install` (or the spec-18 plugin) rewrites the hooks.

## What Changes

- `crates/cortex-adapter-claude-code/src/install.rs` `patch_settings` /
  `build_hook_entry`: emit the array/matcher form per event (matcher `"*"` for
  `PreToolUse` / `PostToolUse`; bare group for the rest). Keep the idempotent
  scan-and-replace so non-cortex hooks survive.
- `uninstall` updated to find + strip the array-form cortex stanza.
- Migration on install: when an existing legacy-object cortex entry is found,
  rewrite it in place to the array form (so a plain `install` heals an old
  config).
- Mirror the format in the spec-18 plugin `hooks/hooks.json` if it carries the
  same shape.

## Impact

- Affected specs: `docs/specs/10-*` (adapter), `docs/specs/18-claude-code-plugin.md`.
- Affected code: `crates/cortex-adapter-claude-code/src/install.rs` (+ tests),
  possibly `packages/cortex-claude-plugin/hooks/`.
- Breaking change: NO (only changes the generated settings shape to the
  supported one).
- User benefit: the live capture pipeline survives Claude Code updates and
  adapter reinstalls instead of silently going dark.
