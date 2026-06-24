## 1. Deterministic doc pass
- [x] 1.1 Scan the doc corpus (`.rulebook/specs/`, `.rulebook/decisions/`, `docs/analysis/`, `CLAUDE.md`/`AGENTS.md`)
- [x] 1.2 Markdown parser emits `document`/`article` + `entity` nodes, parses YAML frontmatter
- [x] 1.3 `[[wikilink]]` extraction emits `related` edges

## 2. Gated LLM annotation
- [x] 2.1 Article-analyzer pass under the phase23c contract emits `claim` nodes
- [x] 2.2 Implicit edges `builds_on`/`contradicts`/`cites`/`exemplifies`/`categorized_under`, explicit-evidence-only, evidence in `description`
- [x] 2.3 Do not duplicate wikilink edges; dedupe entities across the batch

## 3. Decision fusion
- [x] 3.1 Link doc `contradicts` edges against the ADR `SUPERSEDES` chain
- [x] 3.2 Flag the canonical drift case (doc cites superseded ADR while contradicting the current one)

## 4. Topic + pre-thinking integration
- [x] 4.1 Feed `topic` clusters into the existing topic-card system (no parallel topic store) — Entity nodes + schema indexes + CATEGORIZED_UNDER edges serve as the topic-cluster feed; no parallel Topic label needed
- [x] 4.2 Add a fail-open pre-thinking band surfacing doc↔decision contradictions

## 5. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 5.1 Update or create documentation covering the implementation
- [x] 5.2 Write tests: wikilink→related edge; conservative contradicts requires evidence; doc-vs-superseded-ADR drift flagged; pre-thinking band fail-open returns empty on error
- [x] 5.3 Run tests and confirm they pass — 1157/1157 cortex-workers + 153/153 cortex-pre-thinking
