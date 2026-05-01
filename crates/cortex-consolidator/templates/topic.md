# Topic consolidation

You are reviewing several AI coding sessions clustered (HDBSCAN over centroid embeddings, `min_cluster_size = 3`) under a common topic. Produce one evergreen summary that captures what the cluster as a whole established about the topic.

## Inputs

- `topic_label`: `{{topic_label}}` (noun phrase the cluster converged on)
- `repo`: `{{repo}}`
- `cluster_size`: `{{cluster_size}}` sessions
- `temporal_span`: `{{temporal_span}}`
- `outcome_distribution`: `{{outcome_distribution}}`

## Source sessions (one block per session, ordered by `occurred_at`)

```text
{{source_sessions}}
```

## Output contract

Return a JSON object with exactly these fields:

```json
{
  "title": "≤ 80 chars; the topic itself, not 'sessions about X'",
  "summary_markdown": "200–2000 bytes; 3–5 paragraphs; what the cluster collectively established about the topic, what the open questions are, where evidence is strongest vs weakest",
  "takeaways": [
    "5 bullet entries; each must trace to ≥ 1 session in the cluster"
  ]
}
```

Look for cross-session patterns, contradictions, and convergence:
- When two sessions reach different conclusions, name both and the discriminating context.
- When most sessions converge on an approach, name it AND name the dissent.
- Cite ULIDs in takeaways via `(see 01HXSESS…)` so the agent can fetch the source.

Do not invent cross-session takeaways that no individual session supports. If the cluster is heterogenous, say so and emit fewer takeaways instead of forcing structure.
