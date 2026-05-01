//! Intent selector — keyword rule table over the user prompt. Spec
//! 12 §`intent` selection.
//!
//! Pure function, no ML. Adding a rule is one line + a unit test.
//!
//! Phase6d — extended for the `Explain` intent and richer keyword
//! coverage on the existing intents. The selector now returns a
//! [`MatchedIntent`] carrying both the resolved intent AND the
//! keyword that matched, so the audit envelope can record
//! `intent_trigger` and the harness in phase6e can attribute
//! routing changes to the specific rule that fired.

use cortex_api::Intent;

/// One rule in the dispatch table. Matched on lowercased prompt
/// substring; first match wins. `pre_change_context` is the safe
/// default per spec 12.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    /// Substring to look for (case-insensitive).
    pub keyword: &'static str,
    /// Intent the rule maps to.
    pub intent: Intent,
}

/// Phase6d — selector outcome. The trigger field is `None` only
/// when the prompt matched no rule and the selector fell through
/// to the `pre_change_context` default; every explicit match
/// carries the matched keyword verbatim so audit emits stay
/// reproducible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchedIntent {
    /// Resolved intent.
    pub intent: Intent,
    /// Keyword that triggered the match (`None` on default
    /// fallback).
    pub trigger: Option<&'static str>,
}

/// The default rule table, in evaluation order. Spec 12 §intent
/// table. Phase6d — `Explain` first because its keywords
/// (`how does`, `what is`, …) overlap meaningfully with prompts
/// that would otherwise drift to `decision_lookup` ("why did") or
/// the `pre_change_context` fallback. Decision-lookup keywords
/// stay second so policy questions still beat the navigational
/// dispatcher when both signals appear.
pub const DEFAULT_RULES: &[Rule] = &[
    // `Explain` — navigational / explanatory prompts (phase6d).
    // These need to fire BEFORE `decision_lookup`'s "why" rules so
    // a prompt like "explain why we picked X" still routes to
    // explain rather than burning the decisions overlay budget on
    // a code-reading question.
    Rule {
        keyword: "how does",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "what is",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "what's",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "explain",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "show me",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "where is",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "where does",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "find usages",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "find references",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "look up",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "definition of",
        intent: Intent::Explain,
    },
    // `decision_lookup` — questions about prior choices.
    Rule {
        keyword: "why did we pick",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "why do we use",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "history of",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "why did",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "why do",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "who decided",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "should we",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "why is",
        intent: Intent::DecisionLookup,
    },
    // `similar_problems` — debugging signals.
    Rule {
        keyword: "have we seen",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "did we hit",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "stuck",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "keep failing",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "keeps failing",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "kept failing",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "doesn't work",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "doesnt work",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "isn't working",
        intent: Intent::SimilarProblems,
    },
    // `law_check` — policy / permission queries.
    Rule {
        keyword: "is this allowed",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "am i allowed",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "would this violate",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "can i ",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "is it allowed",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "blocked",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "permitted",
        intent: Intent::LawCheck,
    },
    // `pre_change_context` — code-change signals (last because the
    // verbs are common; we still want the table to be deterministic
    // when a prompt contains both kinds of signals).
    Rule {
        keyword: "refactor",
        intent: Intent::PreChangeContext,
    },
    Rule {
        keyword: "modify",
        intent: Intent::PreChangeContext,
    },
    Rule {
        keyword: "rewrite",
        intent: Intent::PreChangeContext,
    },
    Rule {
        keyword: "change",
        intent: Intent::PreChangeContext,
    },
    Rule {
        keyword: "edit",
        intent: Intent::PreChangeContext,
    },
];

/// Phase6d — return both the resolved intent and the keyword that
/// matched. Falls back to `Intent::PreChangeContext` with
/// `trigger = None` when no rule matches.
pub fn select_matched(prompt: &str) -> MatchedIntent {
    select_matched_with(prompt, DEFAULT_RULES)
}

/// Apply [`DEFAULT_RULES`] to `prompt` and return the resolved
/// intent. Falls back to `pre_change_context` per spec 12.
///
/// Wrapper that drops the trigger keyword for callers that don't
/// need it. Prefer [`select_matched`] when audit / observability
/// matters.
pub fn select(prompt: &str) -> Intent {
    select_matched(prompt).intent
}

/// Apply a custom rule table — exposed for tests and operators that
/// want to layer on adapter-specific phrases.
pub fn select_with(prompt: &str, rules: &[Rule]) -> Intent {
    select_matched_with(prompt, rules).intent
}

/// Phase6d — `select_matched`'s ruleset-explicit form.
pub fn select_matched_with(prompt: &str, rules: &[Rule]) -> MatchedIntent {
    let lower = prompt.to_ascii_lowercase();
    for rule in rules {
        if lower.contains(rule.keyword) {
            return MatchedIntent {
                intent: rule.intent,
                trigger: Some(rule.keyword),
            };
        }
    }
    MatchedIntent {
        intent: Intent::PreChangeContext,
        trigger: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refactor_phrase_routes_to_pre_change_context() {
        assert_eq!(
            select("refactor hnsw_search to take ef per call"),
            Intent::PreChangeContext
        );
    }

    #[test]
    fn why_routes_to_decision_lookup() {
        assert_eq!(
            select("why did we pick 128 for ef_search?"),
            Intent::DecisionLookup
        );
    }

    #[test]
    fn stuck_routes_to_similar_problems() {
        assert_eq!(
            select("the recall benchmark keeps failing"),
            Intent::SimilarProblems
        );
    }

    #[test]
    fn can_i_routes_to_law_check() {
        assert_eq!(
            select("can i pass --no-verify to git commit?"),
            Intent::LawCheck
        );
    }

    #[test]
    fn fallback_is_pre_change_context() {
        assert_eq!(select("hi"), Intent::PreChangeContext);
        assert_eq!(select(""), Intent::PreChangeContext);
    }

    #[test]
    fn first_matching_rule_wins() {
        // Phase6d: "explain" fires before any of the why-did rules.
        // A prompt mixing both signals routes to explain — the
        // navigational intent — because the user is asking us to
        // read the answer, not consult a decision record.
        assert_eq!(
            select("explain why we picked hnsw_search?"),
            Intent::Explain
        );
        // Without `explain`, "why did" still beats `refactor`.
        assert_eq!(
            select("why did we refactor hnsw_search?"),
            Intent::DecisionLookup
        );
    }

    // ---- Phase6d new keyword routing tests ----

    #[test]
    fn how_does_routes_to_explain() {
        let m = select_matched("how does the meili fan-out work?");
        assert_eq!(m.intent, Intent::Explain);
        assert_eq!(m.trigger, Some("how does"));
    }

    #[test]
    fn what_is_routes_to_explain() {
        assert_eq!(
            select("what is the lane projection contract?"),
            Intent::Explain
        );
    }

    #[test]
    fn whats_contraction_routes_to_explain() {
        assert_eq!(
            select("what's `LaneHit::normalized_score`?"),
            Intent::Explain
        );
    }

    #[test]
    fn explain_verb_routes_to_explain() {
        assert_eq!(select("explain hnsw indexing"), Intent::Explain);
    }

    #[test]
    fn show_me_routes_to_explain() {
        assert_eq!(select("show me where ef_search is tuned"), Intent::Explain);
    }

    #[test]
    fn where_is_routes_to_explain() {
        assert_eq!(select("where is `RrfFusion` defined?"), Intent::Explain);
    }

    #[test]
    fn where_does_routes_to_explain() {
        assert_eq!(
            select("where does the audit envelope get stamped?"),
            Intent::Explain
        );
    }

    #[test]
    fn find_usages_routes_to_explain() {
        assert_eq!(select("find usages of `derive_decisions`"), Intent::Explain);
    }

    #[test]
    fn find_references_routes_to_explain() {
        assert_eq!(
            select("find references to LANE_EXTRAS_KEYS"),
            Intent::Explain
        );
    }

    #[test]
    fn look_up_routes_to_explain() {
        assert_eq!(select("look up `MatchedIntent`"), Intent::Explain);
    }

    #[test]
    fn definition_of_routes_to_explain() {
        assert_eq!(
            select("show me the definition of `FusionConfig`"),
            Intent::Explain
        );
    }

    #[test]
    fn why_did_we_pick_routes_to_decision_lookup() {
        let m = select_matched("why did we pick the rotate-on-open archive policy?");
        assert_eq!(m.intent, Intent::DecisionLookup);
        assert_eq!(m.trigger, Some("why did we pick"));
    }

    #[test]
    fn why_do_we_use_routes_to_decision_lookup() {
        assert_eq!(
            select("why do we use Synap rooms instead of channels?"),
            Intent::DecisionLookup
        );
    }

    #[test]
    fn history_of_routes_to_decision_lookup() {
        assert_eq!(
            select("history of the cortex-classifier-worker timeout config"),
            Intent::DecisionLookup
        );
    }

    #[test]
    fn have_we_seen_routes_to_similar_problems() {
        let m = select_matched("have we seen this Synap room-not-found error before?");
        assert_eq!(m.intent, Intent::SimilarProblems);
        assert_eq!(m.trigger, Some("have we seen"));
    }

    #[test]
    fn did_we_hit_routes_to_similar_problems() {
        assert_eq!(
            select("did we hit a 401 from vectorizer last week?"),
            Intent::SimilarProblems
        );
    }

    #[test]
    fn is_this_allowed_routes_to_law_check() {
        let m = select_matched("is this allowed under the no-shortcuts rule?");
        assert_eq!(m.intent, Intent::LawCheck);
        assert_eq!(m.trigger, Some("is this allowed"));
    }

    #[test]
    fn am_i_allowed_routes_to_law_check() {
        assert_eq!(
            select("am i allowed to commit with --no-verify?"),
            Intent::LawCheck
        );
    }

    #[test]
    fn would_this_violate_routes_to_law_check() {
        assert_eq!(
            select("would this violate the sequential-editing rule?"),
            Intent::LawCheck
        );
    }

    #[test]
    fn fallback_carries_no_trigger() {
        let m = select_matched("hi");
        assert_eq!(m.intent, Intent::PreChangeContext);
        assert_eq!(m.trigger, None);
    }
}
