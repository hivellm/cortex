# 6. Topic card as living-synthesis vs. consolidation as snapshot

**Status**: proposed
**Date**: 2026-05-03
**Related Tasks**: phase11j_consolidation_tier, phase11r_topic_card_mcp_enrichment

## Context

Phase11j shipped `Kind::Consolidation` — a per-window snapshot ([ADR-005](005-consolidation-grain-choice-session-topic-decisiontrace.md): Session / Topic / DecisionTrace grains) the agent reads in the pre-thinking renderer's `## Consolidated context` section. Each consolidation is keyed by a stable id (`cons-ses-{ulid}`, `cons-top-{ulid}`, `cons-dec-{ulid}`) and is **append-only** — re-running the producer for the same scope mints a new id; old consolidations stay in the lane until the retention sweep tiers them down.

Phase11r needed a higher-priority signal in the renderer's context band: a slug-keyed prose synthesis the orchestrator **rewrites in place** as new evidence accumulates. One slug per topic, one card per `(topic_slug, repo_scope)`. The agent reads ONE living document for "auth-rewrite" instead of N ever-growing snapshots.

Three implementation paths the proposal considered:

1. **Mutate `Kind::Consolidation`** — add a `revision` field, treat the existing `cons-top-*` consolidation as the rewrite target. Producer overwrites the same envelope on each rewrite.

2. **Single living-document `Kind`** — replace `Kind::Consolidation` with one `Kind::LivingDocument` that handles both the snapshot semantics (per-window) and the rewrite semantics (per-topic). One kind, two grains.

3. **Separate `Kind::TopicCard` layered over `Kind::Consolidation`** — keep consolidations append-only as the lower tier; add a new `Kind::TopicCard` whose evidence list cites consolidations + decisions + laws + turns. Each card is a slug-keyed living synthesis.

4. **Separate vault projection** — move topic cards out of the envelope corpus entirely into a sibling vault (e.g. a markdown directory under `docs/cortex/topic-cards/`), updated by a separate writer. Read-side stays a separate code path.

## Decision

Ship a separate `Kind::TopicCard` layered on top of `Kind::Consolidation`. Topic cards cite consolidations as evidence (alongside decisions / laws / turns); consolidations stay append-only as the lower tier. The cards inherit the same Vectorizer / Meili / Nexus routing pattern (per-repo collection + global keyword index + graph node) so the existing pipeline picks them up without a parallel write path.

The two kinds map cleanly:

| Layer            | Kind             | Granularity                       | Mutation        | Lifetime                                |
|------------------|------------------|-----------------------------------|-----------------|-----------------------------------------|
| Living synthesis | `TopicCard`      | One per `(slug, repo_scope)`      | Rewritten in place per revision | Indefinite (revisions accumulate)       |
| Snapshot         | `Consolidation`  | One per window (session/topic/dec)| Append-only     | Retention sweep tiers hot → warm → cold |

Topic cards' `evidence: Vec<EvidenceRef>` carries `EvidenceKind::Consolidation` entries pointing at consolidation ids — a card distills many consolidations into one rolling synthesis. The reverse is not true: consolidations do not cite topic cards (would invert the dependency direction).

## Alternatives Considered

### Mutate `Kind::Consolidation` to be revision-aware

Rejected. Consolidations are anchored on `(grain, scope)` where `scope` is a closed enum — `SessionId(_)`, `Topic(label)`, `DecisionId(_)`. Adding a "live" mode would require a fourth scope variant, splitting the consolidator producers, and rewriting the retention sweep to distinguish snapshots from living docs. The existing append-only contract is also load-bearing for the dashboard's "Consolidated context" timeline view — flipping the same id to a new body breaks the audit trail. Worse, it would entangle the live-rewrite path with the snapshot path: every consolidator producer (`session.rs`, `topic.rs`, `decision_trace.rs`) would gain a "is this a rewrite?" branch, and the cost model would diverge silently per branch.

### Single living-document `Kind`

Rejected. A unified kind would either lose the per-window snapshot semantics (consolidations stop being audit-trail evidence) or carry a discriminator (`mode: Snapshot | Living`) that makes every consumer branch on it. The classifier / fulltext / graph mappers would need exhaustive matches on the discriminator everywhere — a footgun every time someone adds a third mode. Two kinds with one citing the other is structurally cleaner and matches the existing pattern where `Kind::LawViolation` cites `Kind::Law` without merging into one kind.

### Separate vault projection (markdown directory)

Rejected for v1. Moving cards out of the envelope corpus loses every property we built into the pipeline — Synap durability, Vectorizer indexing, Nexus graph membership, the audit envelope contract, the retention tier story. Cortex's central thesis is "everything captured is queryable through the same lanes"; a separate vault would create a second source of truth the dashboard has to reconcile. Useful as a future EXPORT format (the topic-cards CLI can dump a vault snapshot for offline review) but not the primary storage.

### Mutate `Kind::Consolidation` with a `live: bool` flag

A weaker version of alternative #1. Same blast radius (every consumer branches on the flag) plus the additional risk that `live=true` consolidations and `live=false` consolidations land in the same Vectorizer collection — recall queries for "give me one consolidation per session" suddenly need to filter the flag, and a malformed event with `live=true` but no `revision` field is a runtime ambiguity instead of a schema rejection.

## Consequences

**Positive:**
- The `EvidenceKind::Consolidation` arm in the topic card's evidence list makes the dependency direction explicit at the type level — cards depend on consolidations, never the reverse. This is what makes the retention sweep safe: tiering down a consolidation to cold storage does not orphan a card (the card's evidence ref stays resolvable; the cold-tier read path returns a hydrated stub).
- The pre-thinking renderer's section ordering matrix (laws → topic_cards → consolidations → ... ) gets a clean, additive change. The fresh-card path leads with the synthesis; the stale-card path falls back to the consolidation lane the agent already knows. Zero existing code needs to special-case the topic-card section.
- The `derive_topic_card_id(slug, repo_scope)` function produces a stable id per `(slug, repo)` pair so re-runs are idempotent across revisions. Cross-pair collisions are impossible because the slug + scope hash is deterministic. The card's `revision` counter monotonically advances on each rewrite — the audit trail reads `(topic_card_id, revision)` instead of `(consolidation_id_v1, consolidation_id_v2, ...)`.
- The `cortex_topic_diff` MCP tool (§4.4) becomes naturally implementable: walk the parent_event_id chain on the same `topic_card_id` and diff revision N against revision M. With the snapshot model this would have required reconciling N independent ids against a presumed temporal order.
- The cost ledger gets a clean `topic_card` grain bucket alongside the existing `session` / `topic` / `decision_trace` consolidator grains. Operators see where the synthesis budget goes without per-card accounting.

**Negative / tradeoffs:**
- Two kinds doing closely-related work means N consumers (classifier statics, fulltext routing, embedder routing, graph mapper, kind_label, archive_loader) each gain a parallel arm for `Kind::TopicCard`. The match-arm count grows from ~10 to ~11 per consumer — minor, but every new kind adds a fixed-cost step to phase rollout.
- The `EvidenceKind::Consolidation` ref on a topic card is "soft" — the consolidation could be tiered down to warm or cold storage between rewrites. The hydrator (§4.2) handles this gracefully (returns empty title / occurred_at when the source is unreachable); operators reading a hydrated card might see partial citations on older revisions.
- The contradiction detectors are heuristic and run unconditionally on every rewrite. False positives are expected (e.g. a manual `decision_id` rewrite that confuses `DecisionSupersession`). The `status: Open | Reconciled | Deprecated` flow gives operators a way to mark FPs without losing the detector — but absent a dashboard surface for status edits (lands in a follow-up phase), the FPs accumulate as Open contradictions and may make a fresh card look more controversial than it is.
- Future "living document" kinds (e.g. a per-repo `Kind::ProjectREADME`, a per-author `Kind::PersonProfile`) inherit the same blast radius this ADR describes. The pattern is replicable but not free — each new kind goes through the same routing-surface audit. Documented in [`docs/cortex/topic-cards.md`](../../docs/cortex/topic-cards.md) as the canonical template.
