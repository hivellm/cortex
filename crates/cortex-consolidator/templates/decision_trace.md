# Decision-trace consolidation

You are reviewing the parent chain of one `Kind::Decision` envelope (up to 16 hops via `parent_event_id`). Produce an evergreen summary that captures *how* the decision was reached so a future agent can revisit the reasoning without replaying every turn.

## Inputs

- `decision_id`: `{{decision_id}}`
- `decision_title`: `{{decision_title}}`
- `decision_status`: `{{decision_status}}` (proposed / accepted / superseded / deprecated / rejected)
- `repo`: `{{repo}}`
- `decided_at`: `{{decided_at}}`
- `chain_hops`: `{{chain_hops}}`

## Source chain (root → decision, oldest first)

```text
{{source_chain}}
```

## Output contract

Return a JSON object with exactly these fields:

```json
{
  "title": "≤ 80 chars; the decision in active voice — 'Adopt X', 'Switch from A to B'",
  "summary_markdown": "200–2000 bytes; sections: Context (what problem triggered the chain), Forces (what constraints competed), Decision (what was chosen), Alternatives (what was rejected and why)",
  "takeaways": [
    "7 bullet entries; each must trace to ≥ 1 envelope in the chain — at minimum the decision itself + each rejected alternative"
  ]
}
```

Quote the decision body verbatim where it adds signal; otherwise paraphrase. Walk the chain in order so causality is preserved. When the chain crosses an `outcome = blocked_by_law` envelope, surface the law id in the takeaways so future readers see the governance gate.

This grain auto-promotes to Opus (`depth = deep`) — the fidelity threshold (≥ 98 %) holds the bar high. Refuse to invent rationale that the chain does not support; say so in `summary_markdown` if the chain is too sparse to produce 7 takeaways and emit fewer.
