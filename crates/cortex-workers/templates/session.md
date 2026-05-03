# Session consolidation

You are reviewing one session of an AI coding assistant working on a software project. Produce a concise, evergreen summary another agent can read in 30 seconds and walk away knowing what the session accomplished AND what to consult before doing the same work again.

## Inputs

- `session_id`: `{{session_id}}`
- `repo`: `{{repo}}`
- `started_at`: `{{started_at}}`
- `ended_at`: `{{ended_at}}`
- `turn_count`: `{{turn_count}}`
- `outcome_summary`: `{{outcome_summary}}`

## Source turns (`source_event_ids` ordered by `occurred_at`)

```text
{{source_turns}}
```

## Output contract

Return a JSON object with exactly these fields, no preamble:

```json
{
  "title": "≤ 80 chars; what the session was *about* — not 'we did X then Y'",
  "summary_markdown": "200–2000 byte Markdown body; 2–4 paragraphs; mention key files + decisions + dead ends",
  "takeaways": [
    "3 bullet 'lessons learned' entries; each must be verifiable against ≥ 1 source turn"
  ]
}
```

Drop attachment hooks, retry loops, and abandoned approaches. Keep:
- The decision (and the alternatives weighed) when the session converged on one.
- The smallest reproduction of any bug investigated.
- Files / symbols touched (path + symbol; no full diffs).

Refuse to invent supporting evidence. If the session was inconclusive, say so in `summary_markdown` and leave `takeaways` shorter; do not pad.
