---
description: Show the audit envelope for a past Cortex turn or query.
argument-hint: <turn_id_or_query_id>
---

Call the `cortex_query` MCP tool, free-searching the audit
stream for the given id:

```jsonc
{
  "intent": "free_search",
  "query": "query_id:{{ARGS}}",
  "scope": { "topics": ["audit"] },
  "include": ["snippets"],
  "limit": 5,
  "budget_ms": 200
}
```

Render the envelope as:

```text
Audit · {{ARGS}}
  caller:        <claude-code | dashboard | analysis>
  intent:        <pre_change_context | decision_lookup | similar_problems | law_check | free_search>
  scope:
    repo:        <repo>
    files:       <files or "—">
    topics:      <topics or "—">
  counts:
    snippets:    <n>
    decisions:   <n>
    violations:  <n>
    similar:     <n>
    laws_active: <n>
  latency_ms:    <ms>
  cache:         <hit | miss>
  lanes_fired:   <vector | keyword | graph or combinations>
```

If no envelope matches the id, render:
`No audit envelope for "{{ARGS}}" — the turn may not have
issued a Cortex query, or the audit stream rotated past it.`
