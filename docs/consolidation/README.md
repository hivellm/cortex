# Hive Consolidation Knowledge Base

Per-project consolidation snapshots produced for future Cortex ingestion.
Independent of the active task structure (`.rulebook/tasks/`); meant as a
read-only synthesis of each HiveLLM repo as observed on 2026-05-05.

## Layout

```
docs/consolidation/<Project>/
  01 - overview.md
  02 - architecture.md
  03 - public-surface.md
  04 - data-and-storage.md
  05 - integrations.md
  06 - decisions-and-rationale.md
  07 - operational.md
  08 - cortex-relevance.md
  09 - open-questions.md
```

Some projects also have a `README.md` index file (Cortex, Expert,
Transmutation, TransmutationLite). Filenames mix `01 - overview.md` and
`01-overview.md` styles depending on the producing agent — kept as-is to
avoid touching agent outputs post-hoc.

## Projects covered (18)

| Project | Files | Notes |
|---|---|---|
| Assets | 9 | Static brand repo; low ingestion priority |
| CompressionPrompt | 9 | 50% token compression, 89-91% quality (Rust) |
| Cortex | 10 | Self; synthesizes existing `docs/analysis/cortex/` |
| Expert | 10 | Specialized inference on consumer GPUs |
| HiveGPU | 9 | GPU compute / scheduling |
| HivehubCloud | 9 | SaaS control plane (Rust + PostgreSQL) |
| Lexum | 9 | Full-text search (Cortex currently uses Meili) |
| Nexus | 9 | Graph DB / Cypher engine — `2.1.0` external IDs |
| Rulebook | 9 | Rules/specs/task management for AI agents |
| Synap | 9 | KV / Queue / Stream / PubSub in-memory infra |
| Tml | 9 | LLM-friendly language (LL(1), MCP-native) |
| TmlDocs | 9 | Tml docs site + index-only package registry |
| TmlTextmate | 9 | TextMate grammar for `.tml` |
| Transmutation | 10 | Document conversion (Rust, full) |
| TransmutationLite | 10 | TS port for Classify pipeline |
| Umicp | 9 | Universal Matrix Intelligent Communication Protocol |
| Vectorizer | 9 | HNSW vector store + embedding service |
| VectorizerSync | 9 | Vectorizer replication / desktop sync |

Total: 166 markdown files.

## How this was produced

18 `docs-writer` agents (haiku) ran in two parallel waves. Each agent:

1. Read the project README, top-level config, AGENTS.md when present.
2. Globbed source/docs trees, spot-read 2-5 high-signal files.
3. Synthesized into the 9-file template, source-linked with relative paths.

Hard constraint: each agent could only write inside its own
`docs/consolidation/<Project>/` directory. No cross-project edits.

## Top ingestion priorities (aggregate)

Recurring high-value categories Cortex should index first across the
ecosystem:

1. **Decision records** — `.rulebook/decisions/`, `docs/specs/`, `docs/analysis/`
   in each repo. Highest signal-to-noise for "why was X chosen".
2. **Public surfaces** — REST/RPC/MCP/gRPC/SDK signatures and CLI commands.
   Drives `pre_change_context` queries about API shape.
3. **Cross-project contracts** — integration points between Vectorizer,
   Nexus, Synap, Cortex, Rulebook, HivehubCloud. Schemas, ports, auth.

## Known gaps in this snapshot

- File-naming inconsistency across projects (`01 -` vs `01-`).
- A few agents added a `README.md` or `00-index.md` not in the spec.
- `09 - open-questions.md` for Nexus and HiveGPU was filled in a fixup
  pass after the main wave.
- This snapshot is observational — none of it has been pushed through the
  `cortex-bootstrap` or `cortex-walker` ingestion pipeline yet.

## Next steps (suggested, not started)

- Normalize filenames (decide `NN-name.md` or `NN - name.md`).
- Wire `cortex-bootstrap --workspace consolidation.toml` to walk these
  files into per-repo Meili / Vectorizer / Nexus indexes.
- Capture the consolidation pass itself as a Cortex decision/learning
  once ingestion is wired.
