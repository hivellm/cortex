# 01 — graphify architecture (reference)

A condensed, faithful reference so later files can cite stages without re-explaining. Source: graphify `ARCHITECTURE.md`, `docs/how-it-works.md`, and modules under `graphify/`.

## Packaging

Three faces over one core:
- **Agent skill** (`/graphify <path|github-url>`) — the primary UX, shipped for Claude Code, Cursor, OpenCode, Codex, Gemini CLI, Aider, etc. (one `skill-*.md` per tool).
- **Python lib / CLI** (`pip install graphifyy`).
- **MCP server** (`graphify serve`) — exposes graph-query tools over stdio.

## Pipeline (each stage = one module, pure dict/graph in → out, no shared state)

```
detect.py     collect_files()         → classify CODE / DOCUMENT / PAPER / IMAGE / VIDEO (+ incremental manifest)
extract.py    extract()               → Pass1 AST (tree-sitter, ProcessPool); Pass2 A/V transcription (Whisper);
                                         Pass3 semantic LLM (docs/papers/images, multi-backend, SHA256-cached)
ingest.py     ingest()                → remote fetch (arXiv/GitHub/tweet/webpage/PDF), YAML-safe, SSRF-guarded
build.py      build_graph()/merge()   → assemble NetworkX DiGraph; node dedup (AST wins over semantic)
dedup.py      deduplicate_entities()  → entropy gate → MinHash+LSH → Jaro-Winkler → community-boost → union-find
cluster.py    cluster()               → Leiden (graspologic) / Louvain fallback; oversized-split; hub exclusion
analyze.py    analyze()               → god nodes (degree centrality), cross-community "surprises", import cycles
report.py     generate()              → GRAPH_REPORT.md narrative
export.py     export()                → graph.json, graph.html/svg, Mermaid, Obsidian vault
serve.py      start_server()          → MCP query tools (find_node/query/explain/path/similar/communities/compare)
affected.py   affected_nodes()        → BFS impact set for incremental --update
```

## Graph data model

- **Node:** `{id, label, file_type∈{code,document,paper,image,rationale,concept}, source_file, source_location:"L42", community}`. Stable `id` from `make_id()` (language-qualified).
- **Edge:** `{source, target, relation, confidence∈{EXTRACTED,INFERRED,AMBIGUOUS}, confidence_score:0.0–1.0, source_file, source_location, weight}`. `relation` ∈ calls/imports/implements/references/semantically_similar_to/…
- **Hyperedges:** group relations over 3+ nodes (`{label, nodes[], relation, confidence}`).
- **Storage:** NetworkX in-memory → `graph.json` (node-link JSON). SHA256 cache dir. `manifest.json` for incremental state. (Neo4j/FalkorDB are optional export targets, not the primary store.)

## Extraction specifics worth remembering

- **AST first, LLM second.** Code is extracted deterministically (tree-sitter, 25+ langs) → repeatable, zero-token, ground-truth `calls`/`imports`/`defines`. Only prose/images/video hit an LLM. Headline metric: **5.4×–71.5× token reduction** vs. re-reading files per query on mixed corpora.
- **SCIP ingestion** (`scip_ingest.py`): consumes language-server symbol indexes (Rust-analyzer, gopls, tsserver) as JSON → *precise* cross-file references, two-pass (build symbol→id index, then emit edges, stub unresolved as `scip_external` so edges never dangle).
- **Live introspection:** `pg_introspect.py` queries Postgres `information_schema` to reconstruct DDL → SQL extractor; `cargo_introspect.py` walks Cargo workspace members → `crate_depends_on` edges.
- **Confidence tagging:** every edge labeled EXTRACTED (1.0, AST-proven) vs INFERRED (scored rubric 0.55–0.95, LLM) vs AMBIGUOUS — agents/reports can filter by trust.

## Query model (GraphRAG)

- **Local:** `query(question)` → `_score_nodes()` (multi-tier: full-match > token exact > prefix > substring, **IDF-weighted**, +source-file bonus) → `_pick_seeds()` (only nodes scoring > 80% of top, so common terms like "error" don't steal slots) → BFS depth-2 → return subgraph + paths.
- **Global:** `global_add(repo)` merges per-project graphs into `~/.graphify/global-graph.json` for cross-repo questions; community summaries answer "what are the major subsystems".
- **Tools:** find_node, query, explain, path(a,b), similar, communities, compare, neighbors, health.

## Cross-cutting engineering

- **Incremental:** manifest + graph.json present → only changed files re-extracted; `build_merge(prune_sources=deleted)`; manifest written only on success (crash-safe); semantic cache survives renames (updates `source_file` in cached entry).
- **Security** (`security.py`): SSRF guards (private-IP/metadata-endpoint blocking, scheme allow-list), byte caps, office zip-bomb protection, YAML control-char escaping, label sanitization, semantic-fragment schema limits, sensitive-dir blocking (`.ssh`/`.aws`/…).
- **Query log** (`querylog.py`): append-only JSONL, fail-silent, opt-in response capture — usage analytics / learning signal.
