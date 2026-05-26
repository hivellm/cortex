# Living-synthesis trigger heuristic: 3-way OR with cooldown holds

**Category**: architecture
**Tags**: phase11r, trigger, living-synthesis, topic-card, heuristic, pure-function

## Description

A "living document" tier that rewrites in place (rather than appending a new snapshot per window) needs a trigger function that decides Rewrite vs Hold on every incoming event. Phase11r's topic_cards module ships the canonical shape for this: a 3-way OR over independent heuristics + a typed Hold reason for observability.

The trigger fires Rewrite when ANY of:
1. Burst — events_since_last_rev ≥ N (catch sustained activity).
2. High-impact proximity — a new high-impact event lands within distance D AND is one of {Decision, LawViolation, high-impact outcome}.
3. Stale + new evidence — synthesis_age_d ≥ M AND ≥ 1 new evidence cited.

When NONE fire, the trigger returns Hold { reason: HoldReason } where the reason ∈ {Cooldown, LowImpact, NotRelevant}. Holds are silent (no envelope emitted), but the typed reason lets the dashboard surface why a card stayed stale.

The constants (N=8, D=0.30, M=14d in phase11r) are heuristic-tuned for the live corpus; they should be exposed as named constants per file (TRIGGER_EVENTS_THRESHOLD / TRIGGER_DISTANCE_THRESHOLD / TRIGGER_AGE_DAYS) so operators can grep + tune without a code search.

The trigger is a PURE function — takes the current card state + the new event + a "now" timestamp and returns Rewrite | Hold. No I/O, no async, no side-effects. This makes it cheap to unit-test every branch (one test per Rewrite path, one per Hold reason) and lets the orchestrator call it from any thread without lock concerns.

## Example

```rust
pub fn evaluate(
    card: Option<&TopicCardPayload>,
    new_event: &EventClassifier,
    distance: f32,
    now_rfc3339: &str,
) -> TriggerDecision {
    let card = match card {
        None => return TriggerDecision::Rewrite, // first emit always rewrites
        Some(c) => c,
    };

    // Branch 1: burst
    if card.events_since_last_rev >= TRIGGER_EVENTS_THRESHOLD {
        return TriggerDecision::Rewrite;
    }

    // Branch 2: high-impact proximity
    if distance < TRIGGER_DISTANCE_THRESHOLD && new_event.is_high_impact {
        return TriggerDecision::Rewrite;
    }

    // Branch 3: stale + new evidence
    let age_days = days_between(&card.last_rev_at, now_rfc3339);
    if age_days >= TRIGGER_AGE_DAYS && card.events_since_last_rev > 0 {
        return TriggerDecision::Rewrite;
    }

    // Hold with the load-bearing reason
    let reason = if age_days < 1 {
        HoldReason::Cooldown
    } else if !new_event.is_high_impact {
        HoldReason::LowImpact
    } else {
        HoldReason::NotRelevant
    };
    TriggerDecision::Hold { reason }
}
```

## When to Use

Any time you have a "living document" / "rolling synthesis" / "consolidated view" tier that rewrites in place. The pattern decouples the WHEN (trigger) from the HOW (rewrite pipeline) — different signals (burst, proximity, staleness) compose without entangling.

## When NOT to Use

When the document is append-only (snapshot per window) — there's no "rewrite vs hold" decision to make, just emit. Also overkill for documents whose only trigger is a periodic timer (cron-style); a 3-way OR adds branches you'll never exercise.
