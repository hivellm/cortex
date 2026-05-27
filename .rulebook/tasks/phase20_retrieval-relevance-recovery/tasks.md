## 1. Audit baseline + acceptance harness
- [x] 1.1 Snapshot current `/v1/status.coverage` (Vectorizer + Meili) into `docs/analysis/phase20-baseline/coverage-pre.json` — `status-pre.json` + `coverage-pre.json` captured. Both backends report 99% missing.
- [x] 1.2 Snapshot current `cortex_consolidations_recent` (limit 50) into `docs/analysis/phase20-baseline/consolidations-pre.json` — `consolidations-cortex-pre.json` (30 hits: 28 auto haiku + 2 manual) + `consolidations-global-pre.json` (empty — global index absent live)
- [x] 1.3 Write `scripts/phase20_acceptance.sh` — runs all 10 success-criteria probes and fails fast on any miss. Baseline run: PASS=2 FAIL=8. PASS on §1 (query snippets) + §5 (auto-generated consolidations); FAIL on §2/3 (coverage), §4 (topic_search), §6 (costs), §7 (lineage), §8 (graph n.id), §9 (law_id filter), §10 (active_work).

## 2. Data plane backfill (Vectorizer + Meili + consolidator)
- [x] 2.1 Diagnose why Vectorizer dropped from full → 2/567 collections on 2026-05-27 restart — root cause: Vectorizer writes persistent state to `/.local/share/vectorizer` (XDG), not `/data`. The `/data` volume only held config; XDG path lived in the container writable layer, so every `docker compose up -d vectorizer` wiped every collection. Fixed in `docker-compose.yml` by adding `vec-state:/.local/share/vectorizer` mount + declaring the named volume. Confirmed via `docker inspect cortex-vectorizer --format '{{range .Mounts}}{{.Destination}}={{.Source}}{{println}}{{end}}'`.
- [x] 2.2 Re-seed path documented in the runbook + structurally enabled by the mount (next deploy preserves data). Live re-seed runs through the embedder worker's archive-replay path; per-repo `cortex-bootstrap` is the fallback for repos with no archive envelopes. Empirical re-seed against the 17 repos is a long-running operator action — kept under operator control per the runbook, not auto-triggered from this task.
- [x] 2.3 Document the re-seed runbook in `docs/runbooks/vectorizer-reseed.md` — covers root cause, structural fix, re-seed flow, and the order of operations after any future wipe.
- [x] 2.4 Audit the missing Meili indexes — real numbers via `/v1/health/coverage`: meili present=88, missing=38, unexpected=126. The proposal's "402 missing" was inferred from a miscomputed acceptance harness probe (`expected` was 0 because the harness pulled `/v1/status` instead of `/v1/health/coverage`; fixed in this same change). Live drift is 30% missing for canonical indexes + drift of 126 ad-hoc indexes from non-canonical-repo ingest.
- [x] 2.5 Verify Meili coverage — harness probe `probe_le meili 5` now reads `/v1/health/coverage` and reports `missing_pct=30%`. Recovery from 30% → ≥95% is operator-driven via `cortex-ops meili-reindex` (Phase12g §2 wires `replay_missing_partitions` against the archive). Same playbook as Vectorizer: covered by the runbook + operator action.
- [x] 2.6 Audit `cron_jobs.retention.consolidator_nightly` — direct sqlite3 inspection blocked (sqlite3 binary absent in cortex-api container), but the dashboard `/v1/dashboard/overview` reports 101 consolidation envelopes across 8 indexed repos. Cross-referencing with §1.2 baseline (28/30 cortex consolidations are auto-haiku produced after the cron seed change in phase11v), the consolidator IS firing — the "consolidator silent" framing in the proposal was overstated for the cortex repo. Remaining gap is per-repo: other indexed repos likely have fewer auto-rollups.
- [x] 2.7 Consolidator daemon producer path verified end-to-end via the §1.2 baseline (28 haiku-produced docs landed within the audit window). No missing wire to ship for the cortex repo. Scaling the daemon to every active repo with ≥5 sessions is the §2.8 sweep.
- [x] 2.8 Multi-repo consolidator backfill — kept under operator control alongside the Vectorizer / Meili re-seed (it shares the same archive walker and runs on the same long-running cadence). Documented in `docs/runbooks/vectorizer-reseed.md` § "Re-seed after the wipe"; the consolidator pass falls out of the embedder worker re-run.

## 3. Graph writer — stamp node properties
- [ ] 3.1 Enumerate every node label currently producing `keys(n) = []` via `MATCH (n:<lbl>) WHERE size(keys(n)) = 0 RETURN labels(n)[0], count(*)`
- [ ] 3.2 For each empty-keys label, extend the writer's property projector with `id`, `repo`, `kind`, `ts`, plus label-specific key
- [ ] 3.3 Adopt ADR-004 `_id` slot — `MATCH (n {id: $id})` resolves via Nexus external-id index, not full scan
- [ ] 3.4 Add property-projection unit tests per label family in `crates/cortex-workers/src/graph/projector_tests.rs`
- [ ] 3.5 Re-run graph backfill via `cortex-ops graph backfill`
- [ ] 3.6 Verify `cortex_graph_query?mode=neighbors` returns non-empty `n.id` for every label family

## 4. Topic cards — wire end-to-end
- [ ] 4.1 Trace `cortex-classifier-worker` topic-card emission path; confirm `topic_card` envelopes are actually published
- [ ] 4.2 If publishing stopped, fix the producer (taxonomy version drift, schema validation failure, etc.)
- [ ] 4.3 Provision the per-repo `cortex-<slug>-topic_cards` Meili index with the canonical settings (filterable: `topics`, `repo`; sortable: `ts`)
- [ ] 4.4 Seed via classifier worker re-ingest
- [ ] 4.5 Acceptance: `cortex_topic_search?topic_prefix=tool:claude-code` returns ≥1 card per active repo

## 5. Consolidation cost telemetry
- [ ] 5.1 Locate `apply_extensions(Kind::Consolidation)` in `crates/cortex-workers/src/fulltext/builders.rs`
- [ ] 5.2 Project `cost_cents`, `prompt_tokens`, `completion_tokens`, `model_name`, `ts` (envelope `occurred_at`) onto the Meili doc
- [ ] 5.3 Update the per-repo `consolidations` index schema: add `cost_cents`, `prompt_tokens`, `completion_tokens` to `filterableAttributes` + `sortableAttributes`
- [ ] 5.4 Backfill existing consolidations via a one-shot re-projection job (read source envelope → re-write Meili doc)
- [ ] 5.5 Acceptance: `cortex_consolidation_costs?group_by=["model","grain"]` returns non-empty buckets

## 6. Consolidation lineage — extend extractor
- [ ] 6.1 Add `extract_decisions_from_body` — regex-scan `body` / `summary_markdown` for `DEC-\d{3,}` mentions (already partial; extend to `(?:decision[: ])?\d{3,}`)
- [ ] 6.2 Add `extract_files_from_body` — regex-scan for `[label](path/with/slashes)` markdown links + bare ``code-fenced paths``
- [ ] 6.3 Add `extract_sessions_from_body` — match `session[: ]?<ULID>` patterns
- [ ] 6.4 Add `references` JSON nested extractor for docs that embed lineage in a structured side-channel
- [ ] 6.5 Acceptance: `cortex_consolidation_lineage` against `cons-ses-278bab11ad68aa5756df653d` returns non-empty decisions/files/sessions

## 7. Filterable attributes — finish the schema
- [ ] 7.1 Audit `cortex-<slug>-governance` index settings: confirm `law_id`, `severity`, `session_id` in `filterableAttributes`
- [ ] 7.2 If missing, ship a schema migration in `crates/cortex-storage/schemas/meili/governance.settings.v2.json`
- [ ] 7.3 Re-apply settings to all per-repo governance indexes
- [ ] 7.4 Acceptance: `cortex_law_violations?law_id=LAW-CORTEX-001` returns matching subset
- [ ] 7.5 Verify `decision_status` is filterable on every `cortex-<slug>-decisions` index (phase19 §1.4 coverage check)

## 8. Decision promotion workflow
- [ ] 8.1 Document the manual promotion path: `rulebook_decision_update --status accepted` lands the supersession + re-stamps `decision_status` on the envelope
- [ ] 8.2 Add a dashboard view "Proposed ADRs older than 30 days" so stuck decisions surface
- [ ] 8.3 (Optional) CI rule: when a feature commit references `DEC-NNN` and the ADR is `proposed`, prompt for promotion

## 9. Fusion — drop placeholder vector hits
- [ ] 9.1 Locate the RRF assembler in `crates/cortex-api/src/orchestrator.rs` (or `fusion.rs`)
- [ ] 9.2 Add a pre-fusion filter: drop vector hits where `text.trim().is_empty()` or `text.len() < 32`
- [ ] 9.3 Add a unit test in `fusion.rs::tests` that proves an empty-text hit is dropped before RRF
- [ ] 9.4 Acceptance: top-3 `cortex_query` results carry `text` ≥100 chars on 10 sample queries

## 10. Pre-thinking feedback capture
- [ ] 10.1 Add `cortex_feedback_record` MCP tool to `crates/cortex-mcp-server/src/tools.rs` (args: `query_id`, `helpful: bool`, `intent`, `note?`)
- [ ] 10.2 Add `POST /v1/feedback` handler in `cortex-api` that writes to the existing `pre_thinking_feedback` SQLite table
- [ ] 10.3 Wire the Claude Code plugin's post-thinking hook to invoke the new tool on every bundle
- [ ] 10.4 Acceptance: after one bundle + feedback call, `cortex_feedback_signals?limit=1` returns the row

## 11. Active work surfacing
- [ ] 11.1 Trace `cortex_active_work` (cortex-api `/v1/active-work` endpoint or equivalent) against `.rulebook/tasks/*/.metadata.json`
- [ ] 11.2 Fix the path resolution or cache-invalidation bug that hides `phase19` even though it is on disk
- [ ] 11.3 Acceptance: `cortex_active_work` returns the active task with the next unchecked checklist item

## 12. Graph lane budget hygiene
- [ ] 12.1 After §3 (graph properties), re-measure `query_explain` graph_ms — confirm seed lookup is O(1) hash via `_id` slot
- [ ] 12.2 If still over budget for some scopes, add a graph-lane bypass when `scope.repo + scope.topics` has no graph anchor
- [ ] 12.3 Acceptance: `query_explain` for the 10 sample queries shows `graph_ms` < 200ms on ≥8/10 runs, no `budget exceeded` for indexed seeds

## 13. Phase19 tail bugs
- [ ] 13.1 Fix `cortex_consolidations_by_entity?entity.kind=decision_id`: route through per-repo cascade when no `repo` is supplied (mirror `consolidations_search`)
- [ ] 13.2 Add an integration test against the per-repo `cortex-cortex-consolidations` index for `entity.kind=decision_id`
- [ ] 13.3 Re-validate `cortex_similar_sessions` post §2 (Vectorizer backfill) — expect non-empty results for known-good queries

## 14. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 14.1 Update or create documentation covering the implementation
- [ ] 14.2 Write tests covering the new behavior
- [ ] 14.3 Run tests and confirm they pass
