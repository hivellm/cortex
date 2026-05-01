# 05 — Implementation plan (phase 11i)

Six phases, each gated on the previous. Effort estimates assume
single-developer cadence with parallel-where-possible execution
inside each phase. Total: ~3-4 weeks of work.

## Dependencies (must land before §1 starts)

- **phase 11h** — backend coverage `ok`, daemon at HEAD, decisions +
  laws ingested. Without this we'd write the new corpus into a
  half-bootstrapped backend.
- **vectorizer reachable** — coverage gap blocks embedder lane;
  fixed by 11h §2.

## §1 — `cortex-claude-archive` crate (week 1)

New sibling crate at `crates/cortex-claude-archive/`. No git
assumption; walks `~/.claude/projects/` and `~/.codex/sessions/`,
emits canonical `Envelope` JSON to either Synap or
`<CORTEX_ARCHIVE_ROOT>/events/.../bootstrap-claude-NNNNN.parquet`.

**Deliverables:**

1. `Cargo.toml` + crate skeleton with `cortex-core`, `cortex-storage`,
   `serde_json`, `notify`, `zstd`, `tokio` deps.
2. Reader module `src/reader.rs` — streaming JSONL parser tolerant
   to: incomplete final line (active session), out-of-order
   `parentUuid` (load all then resolve), `attachment` records that
   reference a UUID we haven't seen yet.
3. Mapper module `src/mapper.rs` — JSONL records → canonical
   `Envelope`. Handles:
   - matching `user` ↔ `assistant` records by `parentUuid` to form Turn
   - emitting one ToolCall per `assistant.tool_use` block, paired
     with the `attachment.tool_result` that follows
   - sub-agent detection via `Agent` tool name → AgentCall
   - sidecars: `history.jsonl`, `todos/`, `plans/` (Memory /
     Artifact)
4. Walker module `src/walker.rs` — directory traversal with
   `cortex.claude.toml` exclude patterns. CLI flags `--root`
   (defaults to `~/.claude/projects/`), `--projects-only`,
   `--sidecars`, `--codex` (also walk `~/.codex/`).
5. Emitter module `src/emitter.rs` — two sinks: `--sink synap`
   (publish via `cortex-synap` SDK to `cortex.events.bootstrap`)
   and `--sink archive` (write zstd-NDJSON parquet under
   `<CORTEX_ARCHIVE_ROOT>` so `archive_loader` picks it up at
   `cortex-api` boot).
6. Checkpoint module `src/checkpoint.rs` — atomic write of
   `(project_dir, session_id, last_record_uuid, last_byte_offset)`
   every 5 s; honoured by `--resume`.
7. CLI binary `cortex-claude-archive` at
   `crates/cortex-cli/src/bin/cortex-claude-archive.rs`. Subcommands:
   - `bootstrap` — one-shot full ingest with progress bar
   - `tail` — long-running watcher (notify-rs)
   - `estimate` — count + size projection without emitting
8. Redaction wiring — every envelope passes through
   `cortex_core::redact()`; new patterns added in
   `cortex-core/src/redact.rs` for Anthropic / OpenAI / GitHub /
   AWS / Google / JWT shapes.

**Tests (≥95 % coverage gate):**

- 8 unit tests in `mapper.rs` covering each JSONL record type
- 3 fixture-based ITs (tiny / medium / large from §1.4 of the
  inventory) verifying envelope shape + redaction
- 1 watcher IT spawning `tail` against a fake project dir, writing
  records, asserting envelopes appear within 2 s

**Verification:**

`cortex-claude-archive estimate --root ~/.claude/projects/` reports
~9 835 files / ~2.4 M envelopes. `bootstrap --sink archive` finishes
without panic on the full corpus, exits 0, leaves a checkpoint.

## §2 — Classifier + family wiring (week 2, day 1-2)

The new corpus uses existing kinds (Turn / ToolCall / AgentCall) so
families auto-route. The only worker-side change is registering
the new bootstrap kind strings.

1. `crates/cortex-workers/src/classifier/kinds.rs:19` — add
   `"turn.claude-code"`, `"tool_call.claude-code"`,
   `"agent_call.claude-code"` to `kind_from_bootstrap`.
2. `crates/cortex-classifier/src/statics.rs` — topic rule:
   any envelope with `tool == "claude-code"` adds `topics.push("claude-code")`.
3. New IT in `crates/cortex-workers/tests/classifier_claude_archive_it.rs`
   asserting kind / family / topic stamping for one envelope of
   each shape.

## §3 — Relevance axes (week 2 day 3 → week 3)

Five sub-tasks; can ship in any order, each behind its own feature
flag in `cortex-api/config/relevance.toml`.

### §3.1 — Recency decay

- Add `Scope.recency_decay: Option<f32>` (cortex-core types).
- Implement decay multiplier in `cortex-api/src/fusion.rs`.
- IT: `relevance_recency_it.rs` with seeded turns at fixed offsets;
  asserts recent hits outscore older hits at default λ.

### §3.2 — Cross-repo boost

- Add `Scope.cross_repo_boost: f32`.
- Orchestrator forks a parallel lane scan when boost > 0.
- IT: `relevance_cross_repo_it.rs` seeds two repos; query against
  one with boost=0.5 returns hits from both, in-repo first.

### §3.3 — Author + model

- Bump `cortex-workers/src/fulltext/settings.rs` →
  `settings.v2.json`: add `model`, `tool` to filterableAttributes.
- Auto-replay missing settings via `cortex-bootstrap --apply-settings-only`
  (separate flag added here; the existing bootstrap already wires
  `ensure_index`).
- Add `Scope.models`, `Scope.tools` filters in cortex-core types.
- IT: `relevance_model_it.rs` with seeded turns from two models;
  filter respected.

### §3.4 — Session cohesion

- Add `session_id` to Meili filterable.
- `Scope.session_id` + `Scope.session_cohort` in cortex-core.
- Fusion: same-session ×2.0, cohort ×1.5.
- IT: `relevance_session_it.rs`.

### §3.5 — Outcome signal

- Classifier worker derives `Turn.outcome` from child ToolCall
  outcomes + `stop_reason`.
- Add `outcome` to Meili filterable + Vectorizer payload.
- `Scope.outcomes` + `Scope.exclude_outcomes`.
- IT: `relevance_outcome_it.rs`.

### §3.6 — Combined config

- New file `crates/cortex-api/config/relevance.toml` with all
  multiplier defaults.
- `cortex-api` reads it at boot; reload on SIGHUP.
- IT: `relevance_config_reload_it.rs`.

## §4 — Pre-thinking surfaces + measurement (week 3)

1. New section "Past sessions" in
   `crates/cortex-pre-thinking/src/render.rs`:
   one line per session — id, date, first prompt, turn count.
   Top-3 by centroid similarity.
2. Outcome glyph (`✓` / `✗` / `⚠`) in turn / decision lines.
3. Update spec 12 with the new sections.
4. Gold-set measurement:
   - `crates/cortex-api/tests/fixtures/relevance-gold.json` with
     30 hand-curated questions + acceptable result ids.
   - `relevance_eval_it.rs` computes `mrr@10` and `ndcg@10`,
     gated by `CORTEX_RELEVANCE_IT=1`, fails when `mrr@10 < 0.75`.
5. Document the gold-set authoring process in
   `docs/cortex/relevance-tuning.md`.

## §5 — Watcher daemon + ops (week 4 day 1-2)

Long-running `cortex-claude-archive tail` becomes a managed service.

1. `docker-compose.yml` — new service `cortex-claude-archive`
   bind-mounting `~/.claude/projects/` read-only, restarts on
   failure, depends on `synap` + `cortex-ingestion`.
2. Health endpoint: `:17030/healthz` with last-flush timestamp,
   files watched, envelope rate.
3. `/v1/health/coverage` extended: report archive watcher health.
4. Memory budget: hard cap ≤ 512 MiB RSS (Cargo profile + assert
   in IT).
5. Failure mode: corrupt JSONL line → warn, skip, count; never
   panic.

## §6 — Tail (mandatory)

1. CHANGELOG entry summarising 11i.
2. Update `docs/architecture.md` §6 to list the new corpus.
3. Update `docs/specs/16-dashboard.md` so the Memory view surfaces
   the new lane (read-only first; authoring later).
4. `cargo check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test --all-features`, full IT suite green.

## Build sequence (linear order, follows LAW-CORTEX-001)

```
§1.1 → §1.2 → §1.3 → §1.4 → §1.5 → §1.6 → §1.7 → §1.8
   ↓
§2.1 → §2.2 → §2.3
   ↓
§3.1 → §3.2 → §3.3 → §3.4 → §3.5 → §3.6
   ↓
§4.1 → §4.2 → §4.3 → §4.4 → §4.5
   ↓
§5.1 → §5.2 → §5.3 → §5.4 → §5.5
   ↓
§6.1 → §6.2 → §6.3 → §6.4
```

The full task tree is in
[`.rulebook/tasks/phase11i_claude_archive_indexer_and_relevance/tasks.md`](../../../.rulebook/tasks/phase11i_claude_archive_indexer_and_relevance/tasks.md);
the SHALL/Given-When-Then spec lives at
[`.rulebook/tasks/phase11i_claude_archive_indexer_and_relevance/specs/`](../../../.rulebook/tasks/phase11i_claude_archive_indexer_and_relevance/specs/).
