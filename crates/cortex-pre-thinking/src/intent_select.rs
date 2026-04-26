//! Intent selector — keyword rule table over the user prompt. Spec
//! 12 §`intent` selection.
//!
//! Pure function, no ML. Adding a rule is one line + a unit test.

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

/// The default rule table, in evaluation order. Spec 12 §intent
/// table.
pub const DEFAULT_RULES: &[Rule] = &[
    // `decision_lookup` — questions about prior choices.
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

/// Apply [`DEFAULT_RULES`] to `prompt` and return the resolved
/// intent. Falls back to `pre_change_context` per spec 12.
pub fn select(prompt: &str) -> Intent {
    select_with(prompt, DEFAULT_RULES)
}

/// Apply a custom rule table — exposed for tests and operators that
/// want to layer on adapter-specific phrases.
pub fn select_with(prompt: &str, rules: &[Rule]) -> Intent {
    let lower = prompt.to_ascii_lowercase();
    for rule in rules {
        if lower.contains(rule.keyword) {
            return rule.intent;
        }
    }
    Intent::PreChangeContext
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
        // `why did we refactor` triggers BOTH why-did and refactor;
        // the table puts decision_lookup first so the prompt routes
        // there, which matches the spec-12 priority ordering.
        assert_eq!(
            select("why did we refactor hnsw_search?"),
            Intent::DecisionLookup
        );
    }
}
