# 00 — Index & reading order

This analysis compares the external project **graphify**
(`safishamsi/graphify`, branch `v8`, PyPI `graphifyy`) against Cortex's
graph layer and enumerates what Cortex can improve by borrowing from it.

## Reading order

1. [`01-graphify-architecture.md`](./01-graphify-architecture.md) —
   understand what graphify is before comparing.
2. [`02-cortex-vs-graphify.md`](./02-cortex-vs-graphify.md) — the
   capability matrix; the rest of the files expand each gap row.
3. [`03-findings.md`](./03-findings.md) — the enumerated findings
   F-001..F-013, each with evidence (`file:line` for Cortex,
   doc/README references for graphify), impact, and confidence.
4. Topic deep-dives:
   - [`04-language-coverage.md`](./04-language-coverage.md)
   - [`05-graph-analytics.md`](./05-graph-analytics.md)
   - [`06-export-and-visualization.md`](./06-export-and-visualization.md)
   - [`07-token-economics.md`](./07-token-economics.md)
   - [`08-non-code-corpora.md`](./08-non-code-corpora.md)
   - [`09-pr-intelligence.md`](./09-pr-intelligence.md)
5. [`10-execution-plan.md`](./10-execution-plan.md) — phased rollout.

## Prior art in this repo

This analysis builds on the existing internal graph analysis under
[`docs/analysis/graph/`](../graph/README.md), which defined and (in
`phase11k`) implemented Cortex's static code/doc-correlation layer.
That analysis already covered the *internal* gap (missing IMPORTS /
CALLS / MENTIONS edges). **This document covers a different axis:**
what an *external, mature* project does that Cortex still doesn't,
regardless of the phase11k work.

Key distinction: phase11k closed the "we have no code-to-code edges"
gap. Graphify shows the *next* set of gaps — analytics over the graph,
broader language coverage, confidence rubrics, and human-facing
exports — that phase11k did not address.

## Evidence conventions

- **Cortex evidence** is cited as `path:line` against the working tree
  at the time of writing.
- **Graphify evidence** is cited against its `v8` README,
  `ARCHITECTURE.md`, and `docs/how-it-works.md` (URLs in
  [`01-graphify-architecture.md`](./01-graphify-architecture.md)).
- **Confidence** on each finding: `high` (verified in both codebases),
  `medium` (verified one side, inferred other), `low` (inference).
