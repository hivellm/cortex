# Proposal: phase23e_ua-docs-as-graph

## Why

Cortex treats documentation mostly as flat text it fuzzy-matches against. It cannot
say "this note contradicts that accepted ADR" except by luck. Understand-Anything
applies the same graph approach to docs/wikis: a deterministic pass extracts articles,
frontmatter, and `[[wikilinks]]`, then a gated LLM pass surfaces the claims a doc
makes, the entities it names, and whether it agrees with, extends, or contradicts other
docs — materialized as `claim`/`entity`/`topic`/`source` nodes and
`cites`/`contradicts`/`builds_on` edges. This is exactly the living-knowledge contract
the archived `phase11r_topic_card_mcp_enrichment` proposal flagged (Karpathy LLM Wiki /
Obsidian). The highest-value outcome: a `contradicts` edge between a doc and a decision,
letting pre-thinking warn about drift proactively. Depends on the ontology (23a) and
the extraction contract (23c); reuses the markdown parser surface from (23d).

Source: `docs/analysis/understand-anything/08-knowledge-docs.md`,
`docs/analysis/understand-anything/02-findings.md` (F-5).

## What Changes

- Deterministic markdown pass over Cortex's doc corpus (`.rulebook/specs/`,
  `.rulebook/decisions/`, `docs/analysis/`, `CLAUDE.md`/`AGENTS.md`): emit
  `document`/`article` + `entity` nodes, frontmatter, and `[[wikilink]]` → `related`
  edges (support markdown + wikilinks + YAML frontmatter; broader KB-format detection
  is out of scope).
- Gated LLM pass (article-analyzer style, under the phase23c contract): surface
  `claim` nodes and implicit `builds_on`/`contradicts`/`cites`/`exemplifies`/
  `categorized_under` edges — conservative, explicit-evidence-only, evidence stored in
  each edge's `description`; do not duplicate wikilink edges.
- Fuse `contradicts` with the existing ADR `SUPERSEDES` chain: a doc citing a
  superseded ADR while contradicting the current one is the canonical drift alert.
- Feed `topic` clustering into the existing topic-card system (do not build a parallel
  topic system).
- Surface doc↔decision contradictions as a pre-thinking band (fail-open).

## Impact

- Affected specs: docs-as-graph / knowledge ontology population (this task's spec
  delta).
- Affected code: doc ingestion + markdown parser, LLM annotation worker, topic-card
  integration, `cortex-pre-thinking` contradiction band.
- Breaking change: NO (additive knowledge-graph population + an optional pre-thinking
  band).
- User benefit: "is our documentation consistent with our accepted decisions?" becomes
  a graph query, and the model is warned when a recent doc contradicts an accepted ADR.
