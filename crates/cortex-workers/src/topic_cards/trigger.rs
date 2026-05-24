//! Phase11r §2.5 — reactive trigger for topic-card rewrites.
//!
//! [`Trigger::evaluate`] inspects a new event envelope against the current
//! card (if any) and returns a [`TriggerDecision`] indicating whether to
//! rewrite or hold.
//!
//! Heuristics (in evaluation order):
//!
//! 1. **Event count** — rewrite when `events_since_last_rev >= TRIGGER_EVENTS_THRESHOLD`.
//! 2. **High-impact event + proximity** — rewrite when the new event is a
//!    Decision, LawViolation, or high-impact consolidation AND the embedding
//!    distance between the event and the existing synthesis is below
//!    `TRIGGER_DISTANCE_THRESHOLD`.
//! 3. **Age** — rewrite when `synthesis_age_d >= TRIGGER_AGE_DAYS` and any
//!    new evidence lands (i.e. the card exists and has become stale).
//!
//! If none of these fire, the trigger returns `Hold { reason }`.

use cortex_core::events::{Kind, TopicCardPayload};

/// Rewrite when `events_since_last_rev` reaches this threshold.
pub const TRIGGER_EVENTS_THRESHOLD: u32 = 8;
/// Rewrite when the embedding distance between the new event and the existing
/// synthesis is below this value AND the event is high-impact.
pub const TRIGGER_DISTANCE_THRESHOLD: f32 = 0.30;
/// Rewrite when the synthesis is at least this many days old and any new
/// evidence lands.
pub const TRIGGER_AGE_DAYS: u32 = 14;

/// Reason for holding a rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    /// The synthesis is very recent — rewrite would be premature.
    Cooldown,
    /// The incoming event is unlikely to change the synthesis.
    LowImpact,
    /// The event is not semantically relevant to this topic.
    NotRelevant,
}

/// Output of [`Trigger::evaluate`].
#[derive(Debug, Clone, PartialEq)]
pub enum TriggerDecision {
    /// Run the synthesiser now.
    Rewrite,
    /// Hold — do not rewrite yet.
    Hold {
        /// Why the trigger held.
        reason: HoldReason,
    },
}

/// The stateless trigger evaluator.
pub struct Trigger;

impl Trigger {
    /// Decide whether to rewrite the topic card given a new incoming event.
    ///
    /// `card` is `None` when no card has been produced yet — the first-emit
    /// path always rewrites.
    ///
    /// `embedding_distance_fn` returns the cosine distance (in embedding space)
    /// between the new event body and the card's `synthesis_markdown`. A value
    /// closer to `0.0` means the event is highly relevant to the existing
    /// synthesis. Pass a closure returning `1.0` when embeddings are unavailable.
    pub fn evaluate(
        card: Option<&TopicCardPayload>,
        new_event_kind: Kind,
        embedding_distance_fn: &dyn Fn() -> f32,
        synthesis_age_d: u32,
    ) -> TriggerDecision {
        // No card yet — always produce the first revision.
        let Some(card) = card else {
            return TriggerDecision::Rewrite;
        };

        // Rule 1: event-count threshold.
        if card.events_since_last_rev >= TRIGGER_EVENTS_THRESHOLD {
            return TriggerDecision::Rewrite;
        }

        // Rule 2: high-impact event + embedding proximity.
        if is_high_impact(new_event_kind) {
            let dist = embedding_distance_fn();
            if dist < TRIGGER_DISTANCE_THRESHOLD {
                return TriggerDecision::Rewrite;
            }
            // High-impact but distant from the synthesis — not relevant to this card.
            return TriggerDecision::Hold {
                reason: HoldReason::NotRelevant,
            };
        }

        // Rule 3: stale synthesis + any new evidence.
        if synthesis_age_d >= TRIGGER_AGE_DAYS {
            return TriggerDecision::Rewrite;
        }

        // None of the heuristics fired.
        TriggerDecision::Hold {
            reason: HoldReason::LowImpact,
        }
    }
}

/// Returns `true` when the event kind is considered high-impact for the
/// purpose of rule 2.
fn is_high_impact(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Decision | Kind::LawViolation | Kind::Consolidation
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::derive_topic_card_id;

    fn card_with_events(events_since: u32) -> TopicCardPayload {
        TopicCardPayload {
            topic_card_id: derive_topic_card_id("auth", "cortex"),
            topic_slug: "auth".into(),
            repos: vec!["cortex".into()],
            revision: 1,
            synthesis_markdown: "x".repeat(250),
            evidence: vec![],
            contradictions: vec![],
            open_questions: vec![],
            related_topic_ids: vec![],
            confidence: 0.9,
            last_rev_at: "2026-01-01T00:00:00Z".into(),
            events_since_last_rev: events_since,
            synthesis_model: "claude-haiku-4-5".into(),
            synthesis_cost_cents: 5,
        }
    }

    #[test]
    fn no_card_always_rewrites() {
        let decision = Trigger::evaluate(None, Kind::Turn, &|| 1.0, 0);
        assert_eq!(decision, TriggerDecision::Rewrite);
    }

    #[test]
    fn event_count_threshold_fires_at_boundary() {
        let card = card_with_events(TRIGGER_EVENTS_THRESHOLD);
        let d = Trigger::evaluate(Some(&card), Kind::Turn, &|| 1.0, 0);
        assert_eq!(d, TriggerDecision::Rewrite, "at threshold must rewrite");
    }

    #[test]
    fn event_count_below_threshold_does_not_fire() {
        let card = card_with_events(TRIGGER_EVENTS_THRESHOLD - 1);
        let d = Trigger::evaluate(Some(&card), Kind::Turn, &|| 1.0, 0);
        assert!(matches!(d, TriggerDecision::Hold { .. }));
    }

    #[test]
    fn high_impact_event_near_synthesis_fires() {
        let card = card_with_events(0);
        let d = Trigger::evaluate(
            Some(&card),
            Kind::Decision,
            &|| TRIGGER_DISTANCE_THRESHOLD - 0.01,
            0,
        );
        assert_eq!(d, TriggerDecision::Rewrite, "near decision must rewrite");
    }

    #[test]
    fn high_impact_event_far_from_synthesis_holds_not_relevant() {
        let card = card_with_events(0);
        let d = Trigger::evaluate(
            Some(&card),
            Kind::Decision,
            &|| TRIGGER_DISTANCE_THRESHOLD + 0.01,
            0,
        );
        assert_eq!(
            d,
            TriggerDecision::Hold {
                reason: HoldReason::NotRelevant
            }
        );
    }

    #[test]
    fn stale_synthesis_fires_rewrite() {
        let card = card_with_events(0);
        let d = Trigger::evaluate(Some(&card), Kind::Turn, &|| 1.0, TRIGGER_AGE_DAYS);
        assert_eq!(d, TriggerDecision::Rewrite, "stale card must rewrite");
    }

    #[test]
    fn fresh_low_impact_event_holds_with_low_impact_reason() {
        let card = card_with_events(0);
        let d = Trigger::evaluate(Some(&card), Kind::Turn, &|| 1.0, 0);
        assert_eq!(
            d,
            TriggerDecision::Hold {
                reason: HoldReason::LowImpact
            }
        );
    }

    #[test]
    fn law_violation_is_treated_as_high_impact() {
        let card = card_with_events(0);
        let d = Trigger::evaluate(
            Some(&card),
            Kind::LawViolation,
            &|| TRIGGER_DISTANCE_THRESHOLD - 0.01,
            0,
        );
        assert_eq!(
            d,
            TriggerDecision::Rewrite,
            "law violation near card must rewrite"
        );
    }
}
