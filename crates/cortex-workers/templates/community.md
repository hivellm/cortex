# Community consolidation

You are summarising one **graph community** — a cluster of code/architecture entities (files, symbols, modules) that the phase27b Leiden partition grouped together because they are densely connected in the codebase graph. Produce one evergreen subsystem summary an engineer can read to understand what this part of the system is and how it relates to its neighbors.

## Inputs

- `community_id`: `{{community_id}}` (partition id at this hierarchy level)
- `level`: `{{level}}` (0 = coarsest subsystem cut; higher = finer module cut)
- `repo`: `{{repo}}`
- `member_count`: `{{member_count}}` nodes
- `god_nodes`: `{{god_nodes}}` (hub entities excluded from the partition then re-attached — usually the community's most load-bearing names)
- `cross_community_edges`: `{{cross_community_edges}}` (edges leaving this community — how it talks to the rest of the system)

## Member entities (one line per node: `label id name`)

```text
{{members}}
```

## Output contract

Return a JSON object with exactly these fields:

```json
{
  "title": "≤ 80 chars; name the subsystem by what it does, not 'community N'",
  "summary_markdown": "200–2000 bytes; 2–4 paragraphs; what this cluster of entities collectively is (its responsibility), which god nodes anchor it, and how it relates to neighboring communities via the cross-community edges",
  "takeaways": [
    "4 bullet entries; each must trace to member entities or cross-community edges actually present in the input"
  ]
}
```

Ground rules:
- Name the subsystem from the evidence (member names, god nodes), never from guesswork about what the code "probably" does.
- Treat cross-community edges as the interface surface: say which neighboring communities this one calls/imports and which call into it.
- Cite member node names in takeaways so the reader can jump to the entity.
- If the member set is too heterogeneous to be one coherent subsystem, say so explicitly and describe the 2–3 sub-groups you can see instead of forcing one narrative.
