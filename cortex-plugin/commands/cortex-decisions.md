---
description: Look up decisions about a topic — walks the supersession chain.
argument-hint: <topic>
---

Call the `cortex.query` MCP tool with `intent=decision_lookup`:

```jsonc
{
  "intent": "decision_lookup",
  "query": "{{ARGS}}",
  "scope": { "repo": "<repo from cwd>" },
  "include": ["decisions"],
  "budget_ms": 600
}
```

Render every decision in this shape:

```text
- DEC-XXXX (<status>, <YYYY-MM-DD>) — <title>
    Source: <author or analysis id>
    Supersedes: <DEC-id or none>
    Rationale: <one sentence>
```

If the response includes a supersession chain
(`results.decisions[].chain` exposed via the `cortex-historian`
sub-agent's brief), render it as:

```text
History (oldest → newest):
  DEC-2026-008  →  DEC-2026-014 (current)
```

If `decisions` is empty, render `No decisions on file for this
topic.` and stop.
