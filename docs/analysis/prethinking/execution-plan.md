# Pre-Thinking — Execution Plan

> **Analysis ID:** PRE-001 · **Date:** 2026-05-05

Phased plan to maximize Cortex pre-thinking capabilities for LLMs.

---

## Phase 0 — Diagnostics & Baseline (immediate)

### 0.1 — Verify Pre-Thinking Pipeline Health
- [ ] Run `scripts/doctor/health.bat` against running stack; confirm status ≤ degraded
- [ ] Run `scripts/doctor/doctor-config.bat`; confirm no critical findings
- [ ] Run `cortex-ops canary --hook=PostToolUse --deadline-secs=15`; confirm round-trip success
- [ ] Check `/v1/health/freshness` for pipeline stage gaps

### 0.2 — Validate Intent Routing Accuracy
- [ ] Collect 50 real Claude Code prompts from archive (`~/.claude/projects/<project>/*.jsonl`)
- [ ] Run through `intent_select::select_matched()`; manually label correct intents
- [ ] Compute precision per intent; flag any intent with <90% accuracy
- [ ] If `explain` intent misroutes >5% of queries, adjust keyword rules

### 0.3 — Measure Bundle Quality
- [ ] Instrument 20 pre-thinking calls with manual evaluation
- [ ] Score each bundle on: relevance (1-5), actionability (1-5), conciseness (1-5)
- [ ] Correlate low scores with specific context bands (e.g., are snippets irrelevant? decisions stale?)

---

## Phase 1 — Retrieval Quality Tuning (1-2 weeks)

### 1.1 — Tune RRF Parameters Per Intent
- [ ] For `pre_change_context`: test alpha values [0.5, 0.6, 0.7, 0.8]; measure @10 recall on golden set
- [ ] For `similar_problems`: test k values [30, 50, 80]; balance precision vs latency
- [ ] For `explain`: test keyword-first (0.9 keyword weight) vs vector-first splits; measure snippet relevance
- [ ] Lock optimal params per intent in config

### 1.2 — Improve Query Rewriting
- [ ] Enable Sonnet-based rewrite for `pre_change_context` intent (cache-enabled)
- [ ] Measure: does rewritten query improve result Snippet.mean_score by ≥0.05?
- [ ] If yes: keep; if no: revert to deterministic noun-phrase strip

### 1.3 — Validate Classifier Topic Vocabulary Coverage
- [ ] Sample 100 classified events; check if topics align with actual event content
- [ ] Identify missing topics in the ~200-term vocabulary (e.g., "concurrency", "serialization")
- [ ] Add missing topics to both classifier vocabulary AND `topic_for_path()` mapping

---

## Phase 2 — Cognitive Layer Enhancement (2-3 weeks)

### 2.1 — Enable Consolidation Pipeline
- [ ] Verify consolidator worker is processing events into `Kind::Consolidation`
- [ ] Check Meili index `cortex_consolidations` is populated
- [ ] Verify pre-thinking bundle surfaces "Consolidated context" section with real data

### 2.2 — Bootstrap Topic Cards
- [ ] Run `cortex_synthesize` MCP tool on top-10 topic slugs by event volume
- [ ] Verify topic cards land in `cortex.topic_card.fp32` + `cortex_topic_cards` index
- [ ] Check that 3 contradiction detectors fire on known contradictory evidence pairs
- [ ] Verify fresh topic cards appear ahead of consolidations in bundle

### 2.3 — Enable Knowledge + Learnings Source
- [ ] Verify `cortex-bootstrap` walker recurred into `.rulebook/knowledge/**` and `.rulebook/learnings/**`
- [ ] Check that knowledge/learnings embeddings are in Vectorizer `cortex.knowledge.fp32` / `cortex.learning.fp32`
- [ ] Verify these corpora surface in bundles when `intent ∈ {pre_change_context, decision_lookup}`

---

## Phase 3 — Laws & Governance Integration (3-4 weeks)

### 3.1 — Implement Laws DSL
- [ ] Ship spec 13: Markdown + YAML frontmatter law files stored under `laws/`
- [ ] Implement sandboxed Deno detectors for blocking laws (severity=critical)
- [ ] Implement lint-time shape checks for detectors

### 3.2 — Wire Blocking Law Enforcement
- [ ] Implement `PreToolUse` sync law check in Claude Code adapter
- [ ] LAW-007 (no `--no-verify`) must block the tool call, not just annotate
- [ ] Implement observational law capture for non-critical laws (async)

### 3.3 — Enable Punishment Ladder
- [ ] Implement tiered responses: annotation → prompt reminder → blocking → down-weight
- [ ] Compute nightly trust score per (model, repo) from violations + decision adherence
- [ ] Expose trust score via dashboard

---

## Phase 4 — Deep Analysis & Cross-Repo (4-5 weeks)

### 4.1 — Ship Deep Analysis Engine
- [ ] Implement spec 15: multi-agent debate with 2-5 model panel
- [ ] Retrieve all historical context as debate ground truth
- [ ] Capture each round as Turns linked to Analysis node
- [ ] Judge agent (or human) finalizes Decision record

### 4.2 — Cross-Repo Identity Resolution
- [ ] Resolve OQ 5: when same function referenced across repos, deduplicate
- [ ] Content-hash + symbol resolution to unify cross-repo references
- [ ] Update graph writer to create cross-repo edges

### 4.3 — Dashboard Views
- [ ] Ship all 7 dashboard views from spec 16
- [ ] Live timeline with pre-thinking bundle inspection
- [ ] Decision register with supersession graph
- [ ] Law dashboard with violation rates

---

## Phase 5 — Adaptive Tuning & Multi-Adapter (ongoing)

### 5.1 — Adaptive Budgets Per Intent
- [ ] Collect bundle size distributions per intent over 30 days
- [ ] Tune `bundle_bytes` per intent: `similar_problems` gets more snippets (48KB), `explain` gets less (16KB)
- [ ] Tune `time_ms` per intent: `law_check` can be faster (200ms), `pre_change_context` slower (800ms)

### 5.2 — Graduate Intent Routing to ML
- [ ] Only if Phase 0.2 shows >5% precision gap vs rule-based
- [ ] Train lightweight classifier on Haiku-labelled prompt→intent pairs
- [ ] A/B test: rule-based vs ML routing; measure bundle relevance score

### 5.3 — Additional Adapters
- [ ] Ship Cursor adapter (spec 17) — MCP server in `.cursor/mcp.json`
- [ ] Ship Codex/Gemini adapters — HTTP logging proxy
- [ ] Verify pre-thinking bundle works identically across adapters

---

## Success Criteria

| Gate | Metric | Target |
|---|---|---|
| P0 | Pre-thinking consultation rate | ≥ 95% of prompts get non-empty bundle |
| P0 | Hot-path latency P95 | < 150 ms |
| P0 | Fail-open rate | < 1% (timeouts + errors) |
| P1 | Decision adherence rate | ≥ 0.75 (30-day rolling) |
| P1 | Bundle relevance score | ≥ 4.0/5.0 |
| P2 | Topic card freshness | ≥ 80% fresh at query time |
| P2 | Cross-repo resolution accuracy | ≥ 95% |
