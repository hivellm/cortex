# 01 — Graphify architecture

## Sources

- README (`v8`): https://github.com/safishamsi/graphify/blob/v8/README.md
- `ARCHITECTURE.md`: https://github.com/safishamsi/graphify/blob/v8/ARCHITECTURE.md
- `docs/how-it-works.md`: https://github.com/safishamsi/graphify/blob/v8/docs/how-it-works.md

## What it is

Graphify is a **Python library** (`graphifyy` on PyPI, CLI `graphify`)
wrapped as an **AI-assistant skill** installable into ~25 platforms
(Claude Code, Codex, OpenCode, Cursor, Gemini CLI, Copilot, Aider,
Kiro, Antigravity, etc.). The user types `/graphify .` and gets three
files in `graphify-out/`:

- `graph.html` — interactive browser visualization (click/filter/search).
- `GRAPH_REPORT.md` — narrative highlights (god nodes, surprising
  connections, suggested questions, confidence tags).
- `graph.json` — the full graph in NetworkX node-link format.

It is explicitly a **batch snapshotter**, not a live-capture service.
The graph is rebuilt on demand (or on git commit via an installed
post-commit hook) and committed to the repo so a whole team shares one
map.

## The pipeline (ARCHITECTURE.md)

```
detect() → extract() → build_graph() → cluster() → analyze() → report() → export()
```

Each stage is a single pure function in its own module communicating
via plain dicts + NetworkX graphs. No shared state, no side effects
outside `graphify-out/`.

| Module | Responsibility |
|--------|----------------|
| `detect.py` | `collect_files(root)` → filtered file list (respects `.gitignore` + `.graphifyignore`) |
| `extract.py` | per-file → `{nodes, edges}` (tree-sitter for code; LLM for docs/PDF/image) |
| `build.py` | merge extraction dicts → `nx.Graph` |
| `cluster.py` | Leiden community detection → `community` attr per node |
| `analyze.py` | god nodes, surprising connections, suggested questions |
| `report.py` | render `GRAPH_REPORT.md` |
| `export.py` | Obsidian vault / graph.json / graph.html / graph.svg |
| `callflow_html.py` | Mermaid architecture/call-flow HTML |
| `cache.py` | SHA256 semantic cache (skip unchanged files) |
| `serve.py` | MCP server (stdio + Streamable HTTP) over the graph |
| `security.py` | URL/path/label validation |
| `validate.py` | extraction-schema enforcement |

## The three extraction passes (how-it-works.md)

1. **Pass 1 — Code structure (free, local):** tree-sitter parses code,
   extracts classes/functions/imports/call-graphs/inline-comments. **No
   LLM.** 36 grammars. SQL gets deterministic table/view/FK/JOIN
   extraction. If a corpus is code-only, passes 2 and 3 are skipped.
2. **Pass 2 — Video/audio (local):** faster-whisper transcription,
   prompt-seeded with the current top god nodes. Cached.
3. **Pass 3 — Docs/papers/images (LLM, costs tokens):** parallel
   subagents read batches and emit JSON fragments merged into the graph.

## Output schema (ARCHITECTURE.md)

```json
{
  "nodes": [{"id": "...", "label": "...", "source_file": "...", "source_location": "L42"}],
  "edges": [{"source": "...", "target": "...", "relation": "calls|imports|uses|...",
             "confidence": "EXTRACTED|INFERRED|AMBIGUOUS"}]
}
```

`validate.py` enforces this before `build_graph()`. `graph.json` adds
`file_type` (`code|document|paper|image|rationale`), `community`,
`confidence_score` (float, INFERRED only), and `hyperedges` (group
relationships of 3+ nodes) in `G.graph["hyperedges"]`.

## Confidence rubric (how-it-works.md)

| Tag | Meaning | Score |
|-----|---------|-------|
| `EXTRACTED` | explicit in source (import, direct call) | always 1.0 |
| `INFERRED` | reasonable deduction | 0.55–0.95 discrete rubric |
| `AMBIGUOUS` | uncertain → flagged for human review in the report | — |

INFERRED rubric: 0.95 near-certain, 0.85 strong, 0.75 reasonable,
0.65 weak (naming only), 0.55 speculative.

## Key surfaces graphify ships

- **MCP server** (`serve.py`): tools `query_graph`, `get_node`,
  `get_neighbors`, `shortest_path`, `list_prs`, `get_pr_impact`,
  `triage_prs`. Both stdio and Streamable-HTTP transports (one shared
  team server).
- **Exports:** Obsidian, wiki, SVG, GraphML (Gephi/yEd), Neo4j +
  FalkorDB cypher push, callflow-HTML.
- **PR intelligence** (`graphify prs`): CI/review/worktree dashboard,
  per-PR graph impact, AI triage ranking, and "PRs sharing graph
  communities → merge-order risk".
- **Cross-project global graph** (`graphify global …`): register many
  repo graphs into one `~/.graphify/global.json`.
- **Cargo introspection** (`graphify extract --cargo`) and **live
  Postgres introspection** (`--postgres DSN`).

## What graphify deliberately does NOT have

- No live capture of agent sessions / turns / tool calls.
- No bitemporal history (valid-time vs transaction-time).
- No governance / laws / trust scores.
- No vector/embedding lane — community detection uses graph structure
  + LLM `semantically_similar_to` edges instead of embeddings.
- No hybrid fusion across vector+keyword+graph lanes.

These are exactly the dimensions where Cortex is the more capable
system. The comparison in [`02`](./02-cortex-vs-graphify.md) makes the
two-way nature of the gap explicit.
