---
description: Free-text search across everything Cortex captured (vector + keyword + graph fusion via spec-11).
argument-hint: <query>
---

Call the `cortex.query` MCP tool:

```jsonc
{
  "intent": "free_search",
  "query": "{{ARGS}}",
  "scope": { "repo": "<repo from cwd, optional>" },
  "include": ["snippets", "decisions", "similar_turns"],
  "limit": 10,
  "budget_ms": 500
}
```

Render the response as:

- **Snippets** — top 5, format
  `1. \`<repo>/<path>:<symbol>\` — <text first line>` followed
  by 1-2 lines of body.
- **Decisions** — top 3, format
  `- <DEC-id> (<status>) — <title>`.
- **Similar past turns** — top 3, paraphrase the summary; do
  not quote.

If the response carries `debug.errors.<lane>`, mention which
lanes failed at the bottom in muted text. If `debug.truncated`
is `true`, suggest re-running with a larger `budget_ms`.
