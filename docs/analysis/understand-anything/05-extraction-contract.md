# Extraction Contract — Deterministic-Gated LLM Annotation

Spec derived from UA's `file-analyzer` agent + `analyzer/llm-analyzer.ts`. The anti-hallucination
contract is the most portable single idea in UA. See [findings.md](02-findings.md) F-3.

---

## 1. The contract (UA, observed)

Two phases, strictly ordered:

**Phase 1 — Deterministic (`extract-structure.mjs` / extractors + parsers):**
Produces the *authoritative fact set* via tree-sitter (10 code langs) + 12 non-code parsers:
functions, classes, imports, exports, call sites, metrics, line ranges. No LLM.

**Phase 2 — Semantic (LLM):**
Consumes the Phase-1 facts. May only:
- write `summary`, `tags`, `complexity`, `languageNotes` on nodes the facts produced;
- emit semantic edges (`related`, `similar_to`, `depends_on`) **between nodes that already exist**.

**Hard rules (verbatim intent):**
- "NEVER invent file paths. Every `filePath` and every node ID must correspond to a real file."
- "Import edges must enumerate ALL imports from `batchImportData`; output count must equal input count."
- Significance filter: emit a function/class node only if **≥10 lines OR exported**.
- "Do NOT re-read the source files unless the script skipped a file."
- Strict ID prefixes: `file:<path>`, `function:<path>:<name>`, `class:<path>:<name>`.
- Batch outputs: per-batch `batch-<index>.json`; split when nodes > 60 or edges > 120.

---

## 2. Why it works

The LLM never originates structural facts — it only *describes* and *connects* facts the
deterministic pass already proved exist. Hallucinated files/functions are impossible because every
node ID must resolve to a real path, and the import-count equality check (`output == input`) makes
silent omission detectable. The LLM's degrees of freedom are confined to prose + weighted semantic
edges, where being wrong is low-stakes and reviewable.

---

## 3. Cortex port — reconciliation gate

Cortex's adapter/workers that emit graph edges should enforce a reconciliation step between the
extractor and the graph upsert:

```text
facts          = extractor.run(files)        // authoritative: node set N_facts, import list I
annotated      = llm.annotate(facts)         // proposes nodes N_llm, edges E_llm

REJECT annotated.node  if node.id ∉ N_facts.ids          // no invented nodes
REJECT annotated.edge  if edge.source ∉ N_facts ∪ N_known  // endpoints must exist
                        OR edge.target ∉ N_facts ∪ N_known
ASSERT count(edges type=imports for file f) == len(I[f]) // import-count equality, per file
DROP   function/class node if lines < 10 AND not exported // significance filter
NORMALIZE id to prefix scheme; reject malformed ids
```

`N_known` = nodes already in the graph (cross-file edges allowed to point at existing nodes).

**On violation:** drop the offending node/edge, log to the audit envelope (Cortex already has
`cortex_audit`), and — for import-count mismatch — re-run the LLM annotation for that file once
(fail-twice → escalate per project rule), then accept the deterministic import edges directly
(extractor wins) rather than blocking the index.

---

## 4. Tests

| Test | Asserts |
|------|---------|
| seeded hallucinated edge (endpoint not in facts) | rejected, audit logged |
| LLM omits one import | count mismatch detected; deterministic imports backfilled |
| 8-line non-exported helper | filtered out (significance) |
| 8-line **exported** helper | kept |
| malformed node id `foo` (no prefix) | rejected/normalized |

---

## 5. Division of labor (who owns what)

| Output field | Owner | Trust |
|--------------|-------|-------|
| node existence, `filePath`, `lineRange`, imports/exports, call edges | extractor (deterministic) | authoritative |
| `summary`, `tags`, `complexity`, `languageNotes` | LLM | advisory, reviewable |
| `related`/`similar_to`/`depends_on` semantic edges | LLM, gated by reconciliation | accepted only if endpoints exist |
| `weight` on edges | LLM | advisory |

Rule of thumb: **structure is deterministic, meaning is LLM, and meaning may never assert new
structure.**
