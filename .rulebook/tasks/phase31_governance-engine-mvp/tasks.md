## §1. Live-wire the spec-13 evaluator into the PreToolUse sync path (blocking laws)
- [ ] §1.1 Add `cortex-laws` as a dependency of `crates/cortex-api`; load a `LawRegistry` at boot from a configurable directory (env-var driven, empty/missing directory => empty registry per `LawRegistry::load`'s existing behavior — no crash); hold it in `ApiState`.
- [ ] §1.2 Implement `POST /v1/laws/check`: parse `{tool_name, input, session_id, turn_id}`, build a `cortex_laws::EvalContext` (serialize `input` into the flat `args` string the v1 `args_match` regex matches against), call `registry.evaluate()`, filter to `verdict == Deny`, and project each remaining `Verdict` into the adapter's already-expected wire shape `{law_id, severity, message}` (synthesize `message` from `title`/`rationale` since `Verdict` has no `message` field).
- [ ] §1.3 Document (in code comments + this task) the severity-to-blocking mapping decision: v1's shipped `Law` schema has no `mode: blocking|observational` field, so this MVP derives blocking-eligibility purely from `severity == critical`, matching the adapter's existing `severity.eq_ignore_ascii_case("critical")` gate. Do not add a `mode` field unless a concrete need surfaces.
- [ ] §1.4 Confirm the starter law set loaded at rollout has no `severity: critical` entries (or is empty) — `block_on_critical` already defaults to `true` client-side, so §1.2 landing makes any critical law block live sessions immediately; this must be a deliberate choice, not a surprise.

## §2. Live `LawViolation` write path (async, observational)
- [ ] §2.1 New observational consumer in `crates/cortex-workers` (per the existing "cortex-workers as the default host for worker-style daemons" precedent) subscribing to the enriched tool-call event stream; runs `LawRegistry::evaluate()` per event against the same registry snapshot loaded at boot, for laws not eligible for the §1 blocking path.
- [ ] §2.2 On a match, construct a `law_violation`-kind envelope (reuse the schema the 72 bootstrap-imported nodes already prove round-trips through Nexus + the per-repo `cortex-<slug>-governance` Meili index) and POST it through the same ingestion path every other producer uses, so the existing fulltext-worker routing (`Kind::LawViolation => "governance"`) and graph writer pick it up unchanged.
- [ ] §2.3 De-duplicate violations across worker restarts/replay using the same checkpoint-table pattern as ADR-010's `EnvelopeProducer` (keyed on the source event id, not just law id, so repeated evaluation of the same historical event never double-writes).

## §3. Tier-1 / tier-2 punishment ladder
- [ ] §3.1 Tier-1 (dashboard annotation): confirm explicitly that no separate implementation is needed beyond §2 + §5 — a persisted `law_violation` envelope IS the annotation once the dashboard reads live data.
- [ ] §3.2 Tier-2 (reminder injection): add a new SQLite table in `cortex-storage::metadata` (mirrors the existing `pre_thinking_feedback` table's shape) holding pending reminders keyed by `(session_id, law_id)` with a TTL/expiry.
- [ ] §3.3 Wire reminder read-back into the pre-thinking bundle assembly path (`cortex-pre-thinking` / `crates/cortex-adapter-claude-code/src/sync_paths.rs::pre_thinking()`) so an active, non-expired reminder for the session surfaces in the bundle's Laws section on that session's next turn, then is consumed/expired per its `emit_policy`.
- [ ] §3.4 Keep tier-3 (actually blocking via `permissionDecision: deny`) and tier-4 (router down-weighting) out of this task's scope — track them as a separate follow-up task. Document that tier-3's client-side mechanism already exists (§1's Why): excluding it here means the starter law set stays below `critical` severity per §1.4, not that code is left unwritten.

## §4. Nightly trust-score recomputation
- [ ] §4.1 New SQLite table in `cortex-storage::metadata` keyed by `(model, repo)` holding the computed score plus its inputs (violation counts by severity, decisions-followed/contradicted, `last_computed_at`).
- [ ] §4.2 Nightly batch job computing a severity-weighted violation ratio + decision-adherence signal per spec 14 §Trust-score inputs, over the trailing 30 days per `(model, repo)` pair observed in that window.
- [ ] §4.3 On-demand recompute path scoped to a single `(model, repo)` pair.

## §5. Dashboard Laws view → live engine
- [ ] §5.1 New/updated read endpoint(s) exposing the live `LawRegistry`'s active-law catalogue (not derived from violation envelopes) plus the §4 trust scores.
- [ ] §5.2 Update `gui/src/views/Laws.tsx` and the `/v1/dashboard/trust` route (currently a stub) to read from §5.1 instead of the violation-envelope-derived catalogue.

## §6. Live end-to-end verification
- [ ] §6.1 Seed one YAML law using v1's shipped `trigger: {tool, action, args_match}` shape (NOT a TypeScript/Deno detector — that sandbox stays out of this task's scope, per spec 13's own future-phase list) at a non-critical severity with a reproducible trigger.
- [ ] §6.2 Trigger that exact tool-call pattern in a real session; confirm a `LawViolation` envelope is written and retrievable via `cortex_law_violations` immediately afterward.
- [ ] §6.3 Confirm the tier-2 reminder appears in the next `cortex_pre_thinking` bundle assembled for that same session.
- [ ] §6.4 Confirm the dashboard Laws view reflects the live engine's active-law count and the new violation, and the trust route reflects an updated score for the affected `(model, repo)` pair.

## §7. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] §7.1 Update or create documentation covering the implementation (spec 14 status flip to green, dashboard docs, adapter config docs for the laws directory path)
- [ ] §7.2 Write tests covering the new behavior (unit tests for the `/v1/laws/check` handler, the observational consumer, the reminder store, the trust-score job; an IT covering §6's live-fire path)
- [ ] §7.3 Run tests and confirm they pass
