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
/// table. Phase14g — reordered so longer / more-specific compound
/// rules fire before single-word triggers. Two failure modes the
/// reorder closes:
///
/// 1. **Single-word `explain` eating compound decision queries.**
///    "explain why did we pick hnsw" used to route to `Explain`
///    because `explain` matched first; phase14g lifts every
///    compound decision-lookup rule (`why did we pick`, `decided
///    to pick`, `chose to`, `we picked`, …) above the
///    single-word `explain` so the user's actual question wins.
///
/// 2. **Common verbs (`change`, `edit`, `modify`) hijacking
///    pre-change context.** Compound rules like `going to refactor`
///    and `about to change` now fire first, with the bare verb
///    `refactor` / `change` / `edit` as the catch-all fallback.
///
/// The table is grouped by intent for readability but the
/// evaluation order is what matters: longest specific compounds
/// first, then medium-length rules, then single-word fallbacks.
pub const DEFAULT_RULES: &[Rule] = &[
    // ─────────── HIGH-SPECIFICITY COMPOUND RULES ───────────
    // These run BEFORE any single-word triggers. They have ≥3
    // tokens or anchor on a domain-specific phrase the operator
    // would not say casually.

    // decision_lookup — compound phrases that name a prior choice.
    // phase14g §1.2 adds `decided to pick`, `chose to`,
    // `we picked`, `we chose`, `rationale for`, `history behind`
    // covering the observed mismatches.
    Rule {
        keyword: "why did we pick",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "why did we choose",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "why do we use",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "decided to pick",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "chose to",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "we picked",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "we chose",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "rationale for",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "history behind",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "history of",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "who decided",
        intent: Intent::DecisionLookup,
    },
    // similar_problems — compound debugging phrases.
    Rule {
        keyword: "have we seen",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "did we hit",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "broken since",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "regression on",
        intent: Intent::SimilarProblems,
    },
    Rule {
        keyword: "fails intermittently",
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
    // law_check — compound policy / permission queries.
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
        keyword: "is it allowed",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "policy says",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "rules forbid",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "violates law",
        intent: Intent::LawCheck,
    },
    // pre_change_context — compound action phrases. Run before
    // bare verbs so "going to refactor X" is recorded with the
    // compound trigger for audit.
    Rule {
        keyword: "going to refactor",
        intent: Intent::PreChangeContext,
    },
    Rule {
        keyword: "about to change",
        intent: Intent::PreChangeContext,
    },
    // ─────────── EXPLAIN COMPOUNDS ───────────
    // `Explain` compounds run AFTER the compound decision +
    // policy + debug phrases above so a compound question that
    // happens to contain `how does` near a decision keyword still
    // routes to the more specific intent. Single-word `explain`
    // sits in the medium tier below.
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
    // ─────────── MEDIUM-SPECIFICITY RULES ───────────
    // 2-token compounds. Decision-lookup queries lose to
    // navigational `explain` here unless the longer compound
    // (above) already matched.
    Rule {
        keyword: "why did",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "why do",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "why is",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "should we",
        intent: Intent::DecisionLookup,
    },
    Rule {
        keyword: "can i ",
        intent: Intent::LawCheck,
    },
    // ─────────── SINGLE-WORD FALLBACKS ───────────
    // Run last so they only fire when the prompt carries no
    // compound signal. `explain` deliberately sits in this tier
    // (was in the top tier before phase14g) so a compound
    // decision/policy query wins.
    Rule {
        keyword: "explain",
        intent: Intent::Explain,
    },
    Rule {
        keyword: "blocked",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "permitted",
        intent: Intent::LawCheck,
    },
    Rule {
        keyword: "stuck",
        intent: Intent::SimilarProblems,
    },
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
    fn compound_decision_lookup_beats_single_word_explain() {
        // Phase14g §1.4 regression — `"explain why did we pick
        // hnsw"` used to route to Explain because the bare
        // `explain` rule fired before any decision-lookup
        // compound. The reorder puts `why did we pick` ahead of
        // `explain` so a compound decision query lands on
        // decision_lookup as the user intended.
        let m = select_matched("explain why did we pick hnsw");
        assert_eq!(m.intent, Intent::DecisionLookup);
        assert_eq!(m.trigger, Some("why did we pick"));

        // The compound `we picked` also beats explain.
        assert_eq!(
            select("explain why we picked hnsw_search?"),
            Intent::DecisionLookup
        );

        // Without a decision compound, `why did` (medium tier)
        // still beats `refactor` (single-word fallback).
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

    // ───── Phase14g §1.3 — 5 fixture prompts per intent ─────

    #[test]
    fn pre_change_context_fixtures_route_correctly() {
        let prompts = [
            "refactor hnsw_search to take ef per call",
            "modify the budget clipper to accept per-intent caps",
            "rewrite the consolidator nightly entrypoint",
            "going to refactor the lane projection",
            "about to change the synap topic name",
        ];
        for p in prompts {
            assert_eq!(select(p), Intent::PreChangeContext, "prompt: {p}");
        }
    }

    #[test]
    fn decision_lookup_fixtures_route_correctly() {
        let prompts = [
            "why did we pick HNSW over IVF-Flat?",
            "history of the cortex-classifier-worker timeout config",
            "who decided to drop the Cypher gate?",
            "rationale for the 32 KB pre-thinking cap",
            "we chose Synap because of streams, right?",
        ];
        for p in prompts {
            assert_eq!(select(p), Intent::DecisionLookup, "prompt: {p}");
        }
    }

    #[test]
    fn similar_problems_fixtures_route_correctly() {
        let prompts = [
            "have we seen this Synap room-not-found error before?",
            "did we hit a 401 from vectorizer last week?",
            "the recall benchmark keeps failing",
            "broken since the phase14a rebuild",
            "the producer test fails intermittently after restart",
        ];
        for p in prompts {
            assert_eq!(select(p), Intent::SimilarProblems, "prompt: {p}");
        }
    }

    #[test]
    fn law_check_fixtures_route_correctly() {
        let prompts = [
            "is this allowed under the no-shortcuts rule?",
            "am i allowed to commit with --no-verify?",
            "would this violate the sequential-editing rule?",
            "the policy says I cannot rebase published commits",
            "can i pass --no-verify to git commit?",
        ];
        for p in prompts {
            assert_eq!(select(p), Intent::LawCheck, "prompt: {p}");
        }
    }

    #[test]
    fn explain_fixtures_route_correctly() {
        let prompts = [
            "how does the meili fan-out work?",
            "what is the lane projection contract?",
            "show me where ef_search is tuned",
            "where does the audit envelope get stamped?",
            "find usages of derive_decisions",
        ];
        for p in prompts {
            assert_eq!(select(p), Intent::Explain, "prompt: {p}");
        }
    }

    #[test]
    fn explain_single_word_falls_through_when_no_compound_decision() {
        // Bare `explain` with no decision compound still routes
        // to Explain.
        assert_eq!(select("explain hnsw indexing"), Intent::Explain);
    }
}
