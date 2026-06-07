---
description: List active laws in the current scope (repo + optional topics).
argument-hint: [topic ...]
---

Call the `cortex_query` MCP tool with `intent=law_check`:

```jsonc
{
  "intent": "law_check",
  "query": "{{ARGS}}",
  "scope": {
    "repo": "<repo from cwd>",
    "topics": [{{ARGS_AS_TOPIC_LIST_OR_EMPTY}}]
  },
  "include": ["violations"],
  "budget_ms": 300
}
```

Render the response in this exact shape:

```text
Active laws · {{REPO}}
  · LAW-007 (critical · block) — Never bypass pre-commit hooks
  · LAW-012 (notable · observe) — HNSW recall benchmarks must run before merge
  ...

Recent violations (last 7d):
  · VIO-... (LAW-007, critical) — <message> · <observed_in>
  · ...
```

If `laws_active` is empty, render `No laws codified for this
scope yet.` and stop.

If `violations` is empty, omit the section entirely.
