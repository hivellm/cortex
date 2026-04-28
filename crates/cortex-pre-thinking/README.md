# cortex-pre-thinking

> Spec: [`docs/specs/12-pre-thinking-injection.md`](../../docs/specs/12-pre-thinking-injection.md)

Bundle assembly for adapter-side pre-thinking injection. Turns the
raw retrieval bundle returned by `cortex-api /v1/query` into a
compact, deterministic Markdown block that an adapter can drop into
the model's system prompt before it plans a response.

```
cortex-api /v1/query ──▶ cortex-pre-thinking ──▶ Markdown block
                          (heuristics, caps,        ▼
                           byte budget)        adapter system prompt
```

The crate is **library only**. The hook contracts that POST to it
live in [`cortex-adapter-claude-code`](../cortex-adapter-claude-code/);
the MCP `cortexPreThinking` tool that exposes it externally lives in
[`cortex-mcp-server`](../cortex-mcp-server/).

## What it owns

- **Scope-derivation heuristics** — turn a user prompt + cwd +
  recent files into the `scope` field on a `cortex-api` query.
- **Bundle formatter** — deterministic Markdown; no model-generated
  prose. Same input always produces the same output.
- **Byte-budget enforcement** — caps the bundle at
  `adapter.pre_thinking.max_bundle_kb` (default 32 KB).
- **Per-section caps** — N decisions, N similar turns, N snippets,
  N laws, with fairness so one section cannot crowd out the rest.
- **Debug tracing** — every assembled bundle carries a `query_id`
  so retrieval-quality analysis can be done after the fact.

## What it does *not* own

- Hook wiring (in `cortex-adapter-claude-code`).
- Query-lane orchestration / RRF fusion (in `cortex-api`).
- Evaluation harness / offline scoring (Phase-4 hardening item).
- Non-Claude-Code adapters (`cortex-adapter-cursor`, etc. — Phase 3).

## Library

```toml
[dependencies]
cortex-pre-thinking = { path = "../cortex-pre-thinking" }
```

```rust
use cortex_pre_thinking::{assemble_bundle, PreThinkingInput, PreThinkingBudget};

let input = PreThinkingInput {
    session_id: &session_id,
    turn_id: &turn_id,
    user_prompt: prompt,
    cwd: &cwd,
    recent_files,
    budget: PreThinkingBudget::default(),
};
let block = assemble_bundle(&query_response, &input)?;
```

## Tests

```bash
cargo test -p cortex-pre-thinking
```

Unit tests cover the formatter (deterministic Markdown), the budget
enforcer (byte cap, per-section fairness), and the scope-derivation
heuristics on a corpus of fixture prompts.

## Stability

Pre-1.0. The Markdown shape is the load-bearing contract because
adapters cache it verbatim into prompt history. Major changes go
through the same review path as `cortex-core` envelope changes.
