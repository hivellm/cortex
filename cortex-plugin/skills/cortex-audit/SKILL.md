---
name: cortex-audit
description: Fetch the audit envelope for a past Cortex turn or query. Returns caller, intent, scope, per-field counts, latency, cache hit/miss, and a structured trace of what each retrieval lane returned. Use this when the user asks "what did Cortex do?", "why did the model see X?", or wants to debug retrieval quality on a specific turn.
---

# cortex-audit

Use this skill to debug retrieval — what Cortex *did* on a past
turn, not what the model did with it.

## When to use

- The user asks "why did Cortex return X?" / "what context did
  the model see?" / "audit turn `<id>`".
- The model just produced a result that looks wrong, and the user
  wants to check whether the bad output came from Cortex retrieval
  or the model itself.
- A law violation surfaced; the user wants to confirm the audit
  envelope captured it.

Do NOT use this skill to:
- Pull fresh context for the current task — use `cortex-context`.
- Search for events — use `cortex_query` with `intent=free_search`.

## How to invoke

Call `cortex_query` with the `audit` intent's substitute (free
search against the audit stream):

```jsonc
{
  "intent": "free_search",
  "query": "query_id:<turn_id>",
  "include": ["snippets"],
  "scope": { "topics": ["audit"] },
  "budget_ms": 200
}
```

The audit stream (`cortex.events.query_audit`) carries one
envelope per query. The `query_id` is what the spec-12 bundle's
leading comment surfaced — copy it from there.

## How to use the response

The audit envelope carries:

- `caller` — `claude-code` / `dashboard` / `analysis`.
- `intent` — `pre_change_context` / `decision_lookup` /
  `similar_problems` / `law_check` / `free_search`.
- `scope` — repo, files, topics, since.
- `counts` — per-field result counts (snippets, decisions,
  violations, graph_neighbors, similar_turns, laws_active).
- `latency_ms` — total wall-clock.
- `cache` — `hit` / `miss`.

Summarise: "Cortex ran a `<intent>` against scope `<scope>` and
returned `<count>` results in `<latency>` ms (cache: `<state>`).
The retrieval lanes that fired were: `<lanes>`."

If the audit shows `reason: scope_forbidden` or `rate_limited`,
flag it explicitly — the model didn't see the bundle the user
might have expected.

## Empty result

If no audit envelope matches the `query_id`, either the turn
didn't issue a Cortex query (pre-thinking was disabled or the
adapter wasn't running) or the audit stream rotated past it.
Note both possibilities to the user.
