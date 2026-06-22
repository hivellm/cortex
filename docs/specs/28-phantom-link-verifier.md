# 28 — Phantom-Link Verifier

> **Status:** ✅ P3 shipped · **Owner:** Core team · **Depends on:** 11 (fusion lane), 27 (reranker — same phase)
> **Phase:** phase17_cdc-code-doc-correlation

## Goal

Check whether a cited `(path, symbol)` pair in a retrieved snippet actually
exists in the repository before surfacing it to the model. Stale code
references ("phantom links") degrade model accuracy by citing functions that
have been renamed or files that have been deleted. The verifier attaches
`verified: bool` and `verdict: SymbolVerdict` metadata to every snippet and,
in `"filter"` mode, removes unverified snippets entirely.

## Scope

**In:**

- `SymbolVerdict` enum (`Verified`, `NotFound`, `FileMissing`,
  `Unsupported`) with `serde` derives — in
  `crates/cortex-workers/src/verify/symbols.rs`.
- `verify_symbol(path: &Path, symbol: &str) -> SymbolVerdict` function
  (dispatches to Rust or Markdown resolver).
- **Rust resolver (§3.3)**: Tree-sitter parse of `.rs` files, recursive
  walk of top-level items (fn/struct/enum/trait/impl/mod/type/const/static).
- **Markdown resolver (§3.4)**: String scan of `.md` files — ATX heading
  slugs (GitHub anchor format) and code-fence identifier lines.
- **LRU file-content cache (§3.7)**: `Mutex<LruCache<PathBuf, Arc<String>>>`
  with 1 000 entries; avoids repeated disk I/O on hot retrieval paths.
- `VerifyConfig { symbols_enabled, action }` in `cortex-config` + `Config`
  field `verify`. Env knobs: `CORTEX_VERIFY_SYMBOLS_ENABLED`,
  `CORTEX_VERIFY_ACTION`.
- Post-snippet-assembly pass in `Orchestrator::run` — runs when
  `verify_cfg.symbols_enabled = true` and `workspace_root` is set.
  Attaches `verified` and `verdict` to each snippet with a non-`None`
  `(path, symbol)` pair.
- `Orchestrator::with_verify(VerifyConfig, PathBuf)` builder method.
- **Audit event (§3.9)**: `tracing::info!(target: "cortex_audit", event =
  "phantom_link_dropped", dropped = N, action = action, query_id = ...)` when
  any snippets are unverified or filtered.
- Unit tests (§3.8): present symbol, renamed symbol, deleted file,
  unsupported language, markdown heading, code-fence identifier — all in
  `crates/cortex-workers/src/verify/symbols.rs`.

**Out:**

- Go/Python/TypeScript resolvers: Unsupported for now; extend by adding a
  resolver branch in `verify_symbol` and a new tree-sitter grammar call.
- Per-symbol caching (bypass LRU for same path+symbol combo): future work.
- Phantom-rate metric in `cortex-eval` (≤ 1% gate): the retrieval suite
  measures MRR/recall, not phantom rate; a dedicated harness addition is
  needed to count `verified=false` snippets per query.

## Live status (phase0, 2026-06-22)

The dead-code wiring gap (§3.10 originally blocked) was closed in phase0.
`main.rs` now calls `with_verify(cfg.verify, root)` at boot — the same
root cause as the reranker (`with_verify` existed but was never called).

**Current config on cortex-api:**

| Knob | Value |
|------|-------|
| `CORTEX_VERIFY_SYMBOLS_ENABLED` | `true` |
| `CORTEX_VERIFY_ROOT` | `/workspaces/Cortex` (source bind-mounted) |
| `CORTEX_VERIFY_ACTION` | `flag` |

**Verified live:** log line `phantom-link verifier wired` on boot;
snippets carrying a `symbol` field receive `verified`/`verdict` metadata
(e.g. `verified=false verdict=not_found`); symbol-less snippets pass
through with `verified=null` (not checked). See commit 2ca7970.

Phantom-link **rate** gate (≤ 1%) still requires a dedicated metric pass
in `cortex-eval`. The flag-mode data is flowing; measuring the rate needs
a harness that counts `verified=false` snippets across the golden set.

## Config defaults

| Field | Default | Env override |
|-------|---------|-------------|
| `symbols_enabled` | `true` | `CORTEX_VERIFY_SYMBOLS_ENABLED` |
| `action` | `"flag"` | `CORTEX_VERIFY_ACTION` |

`"flag"` — attach `verified = false` metadata without dropping snippets.
Recommended for the first 2 weeks to measure phantom-link rate without
affecting retrieval. Switch to `"filter"` once confidence is established.

## Actions

| `action` | Behaviour |
|----------|-----------|
| `"flag"` | Attach `verified: false`, emit audit event, keep snippet. |
| `"filter"` | Same as `"flag"`, then remove unverified snippets from results. |

## Snippet wire shape

```json
{
  "rank": 1,
  "path": "crates/cortex-api/src/search/orchestrator.rs",
  "symbol": "Orchestrator",
  "text": "...",
  "verified": true,
  "verdict": "verified"
}
```

`verified` and `verdict` are `null` (absent) when:
- The verifier is disabled (`symbols_enabled = false`).
- No `workspace_root` is configured.
- The snippet has no `path` or no `symbol`.
- The language is unsupported.

## Audit event

```
event = "phantom_link_dropped"
dropped = N
action = "flag" | "filter"
query_id = "<uuid>"
```

Emitted once per query when at least one snippet has `verified = false`.
The `dropped` counter reflects snippets removed in `"filter"` mode and
snippets flagged (but kept) in `"flag"` mode.

## Supported languages

| Extension | Resolver | Matches |
|-----------|----------|---------|
| `.rs` | Tree-sitter (Rust) | fn, struct, enum, trait, impl, mod, type, const, static |
| `.md` | String scan | ATX heading anchors, code-fence item identifiers |
| Other | — | `Unsupported` (verdict set, no drop/flag) |
