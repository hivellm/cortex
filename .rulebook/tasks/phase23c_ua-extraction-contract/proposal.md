# Proposal: phase23c_ua-extraction-contract

## Why

When an LLM emits graph nodes and edges directly, it can hallucinate files, functions,
or relationships that do not exist — poisoning the graph that pre-thinking later trusts.
Understand-Anything solves this with a two-phase contract: a deterministic extractor
produces the authoritative fact set, and the LLM may only annotate those facts (write
summaries/tags, connect existing nodes) — never originate structure. A reconciliation
gate enforces it: every emitted node/edge endpoint must exist in the fact set, and the
import count must reconcile exactly. This is the most portable anti-hallucination
pattern in UA and directly strengthens graph trustworthiness. It depends on the
ontology (phase23a) and complements the incremental indexer (phase23b).

Source: `docs/analysis/understand-anything/05-extraction-contract.md`,
`docs/analysis/understand-anything/02-findings.md` (F-3).

## What Changes

- Split graph extraction into Phase 1 (deterministic facts: nodes, imports/exports,
  call edges, line ranges — no LLM) and Phase 2 (LLM annotation: summary, tags,
  complexity, semantic edges between existing nodes).
- Add a reconciliation gate between Phase 2 and graph upsert:
  - reject any node whose id is not in the fact set;
  - reject any edge whose `source`/`target` is not in the fact set or the existing
    graph;
  - assert per-file import-edge count equals the deterministic import count;
  - drop function/class nodes below the significance filter (≥10 lines OR exported);
  - normalize/reject malformed node ids (strict prefix scheme).
- On violation: drop the offending item, log to the audit envelope; on import-count
  mismatch, re-run annotation once (fail-twice → escalate), then accept the
  deterministic import edges directly (extractor wins).

## Impact

- Affected specs: extraction contract / reconciliation gate (this task's spec delta).
- Affected code: `crates/cortex-workers` (or adapter) extraction path, audit-envelope
  emit for rejections.
- Breaking change: NO (tightens an internal path; output graph becomes a subset of
  what an ungated LLM would emit).
- User benefit: a graph free of invented files/edges; every semantic edge carries
  evidence and reconciles against deterministic facts.
