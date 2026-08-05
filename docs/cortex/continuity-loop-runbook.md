# Continuity-loop runbook — "the agent forgot what we just did"

Phase30 (continuity-loop-verification §1.4). Operator triage for
cross-session memory complaints: a prior session's work should reach a
fresh session through TWO independent channels. Check them in order —
each has a distinct failure signature.

## The two channels

1. **Active-work surfacing (session start).** The adapter's
   `SessionStart` hook fetches `GET /v1/dashboard/active-work?repo=<slug>`
   and injects an "active work" block listing in-flight Rulebook tasks
   (id, status, next unchecked item). Source of truth:
   `.rulebook/tasks/` on disk — no indexing pipeline involved.
2. **Consolidated context (per prompt).** Every `UserPromptSubmit` runs
   the spec-12 pre-thinking pipeline: `POST /v1/query` → bundle sections
   (topic cards → consolidations → past sessions → snippets). This is
   the channel that carries a prior session's *distillate* — and it
   depends on the whole ingestion → classifier → indexer pipeline.

## Triage checklist

### 0. Scope sanity (30 s)

- Same `repo`? Both channels scope by repo slug. A session started in a
  different cwd resolves a different slug and legitimately sees nothing.

### 1. Active-work channel

- `curl -s "$API/v1/dashboard/active-work?repo=<slug>"` — rows present?
  - **No rows**: the prior session never tracked its work in
    `.rulebook/tasks/` (nothing to surface — not a bug), or the wrong
    repo filter.
  - **Rows but no block in the new session**: adapter daemon down or
    stale (check its `/healthz` on :17011), or the SessionStart hook is
    not installed (`cortex-adapter-claude-code install` state).
- The MCP fallback always works interactively: call
  `cortex_active_work` and compare.

### 2. Consolidated-context channel

Walk the pipeline in write order; stop at the first broken stage.

- **Was a consolidation ever produced?**
  `curl -s "$API/v1/consolidations/recent?repo=<slug>&limit=5"` —
  the prior session's distillate should be here. If not: the
  consolidator hasn't run over that session yet (check the
  consolidation cron / `cortex-ops` consolidation surface —
  see `docs/cortex/consolidation-tuning.md`).
- **Does retrieval return it?** `POST /v1/query` with intent
  `pre_change_context`, a prompt matching the consolidation's topic, and
  `scope.repo=<slug>` — inspect `results.consolidations`.
  - **KNOWN BREAK (2026-08-05, tracked as `phase30b_consolidations-read-path`):**
    this is currently ALWAYS empty — the read path queries an extinct
    global Meili index (`cortex_consolidations`; writes go per-repo to
    `cortex-<repo>-consolidations`), the `cortex.consolidation.fp32`
    vector collection does not exist, and cortex-api never assembles
    `ConsolidationRef`s. Until phase30b lands, channel 2 carries
    consolidations ONLY via generic snippets (if at all) — complaints
    about forgotten *conclusions* are expected and are THIS bug, not an
    operator problem.
- **Does the bundle render it?** The acceptance probe for the whole
  channel is the live-gated IT:
  `cargo test -p cortex-pre-thinking --test cross_session_continuity_it -- --ignored`
  (green ⇒ loop verified end-to-end; it is `#[ignore]`d while phase30b
  is open).

### 3. Pipeline freshness (when stage 2 shows stale data)

- Worker health: `:17021`/`:17022`/`:17023`/`:17024` `/healthz` —
  `degraded (idle)` after a synap restart may be the stale-cursor class
  (phase29c self-heal handles `cursor > head`; the `cursor == head`
  coincidence is phase29d / synap#257).
- Full drainage triage: `docs/cortex/pipeline-drainage-runbook.md`.
- Umbrella: `cortex-ops doctor-smoke` (exercises every read MCP tool
  against the live api).

## Quick reference

| Symptom | First check | Likely owner |
|---|---|---|
| No active-work block at session start | adapter `/healthz` :17011; hook installed | adapter daemon |
| Active-work block empty but tasks exist | `?repo=` slug mismatch (cwd) | operator/config |
| Prior session's conclusions never surface | `results.consolidations` empty | **phase30b (known)** |
| Conclusions stale (old but not latest) | consolidation cron lag | consolidator |
| NOTHING surfaces, all intents | workers degraded / rooms wiped | pipeline (phase29c/d) |
