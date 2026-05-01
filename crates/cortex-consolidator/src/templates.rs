//! Phase11j §2.2 — prompt templates per grain.
//!
//! Three Markdown templates (one per grain) live as `include_str!`
//! constants under `crates/cortex-consolidator/templates/`. Each
//! template declares an input-format spec (the `{{key}}` slots the
//! producer must fill), a max-output-bytes cap, and a required
//! `takeaways[]` count. The producer renders the template against
//! a `(key, value)` mapping and ships the rendered text through a
//! [`crate::summariser::Summariser`] impl.

use cortex_core::events::ConsolidationGrain;

/// Per-template metadata the producer reads at render time.
#[derive(Debug, Clone)]
pub struct Template {
    /// Grain this template targets.
    pub grain: ConsolidationGrain,
    /// Raw Markdown body (rendered against the producer's input
    /// via plain `{{key}}` substitution).
    pub body: &'static str,
    /// Byte cap on the model's response. The summariser layer
    /// passes this through as `max_output_tokens` divided by ~4.
    pub max_output_bytes: u32,
    /// Required number of takeaways (`takeaways[]` length on the
    /// resulting payload). The fidelity IT (§6.2) reads this to
    /// drive the per-takeaway support check.
    pub takeaways_count: u32,
}

/// Phase11j §2.2 — Session producer prompt body.
pub const SESSION_TEMPLATE_BODY: &str = include_str!("../templates/session.md");
/// Topic producer prompt body (HDBSCAN-clustered turns).
pub const TOPIC_TEMPLATE_BODY: &str = include_str!("../templates/topic.md");
/// Decision-trace producer prompt body.
pub const DECISION_TRACE_TEMPLATE_BODY: &str = include_str!("../templates/decision_trace.md");

impl Template {
    /// Resolve the template for a grain.
    pub fn for_grain(grain: ConsolidationGrain) -> Self {
        match grain {
            ConsolidationGrain::Session => Self {
                grain,
                body: SESSION_TEMPLATE_BODY,
                max_output_bytes: 2_000,
                takeaways_count: 3,
            },
            ConsolidationGrain::Topic => Self {
                grain,
                body: TOPIC_TEMPLATE_BODY,
                max_output_bytes: 2_000,
                takeaways_count: 5,
            },
            ConsolidationGrain::DecisionTrace => Self {
                grain,
                body: DECISION_TRACE_TEMPLATE_BODY,
                max_output_bytes: 2_000,
                takeaways_count: 7,
            },
        }
    }

    /// Substitute `{{key}}` slots against a key/value mapping.
    /// Conservative: anything that does not match a key in the map
    /// passes through verbatim so a typo surfaces in the rendered
    /// prompt rather than dropping silently.
    pub fn render<'a, I>(&self, vars: I) -> String
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut out = self.body.to_string();
        for (key, value) in vars {
            let needle = format!("{{{{{key}}}}}");
            out = out.replace(&needle, value);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_grain_returns_distinct_templates() {
        let s = Template::for_grain(ConsolidationGrain::Session);
        let t = Template::for_grain(ConsolidationGrain::Topic);
        let d = Template::for_grain(ConsolidationGrain::DecisionTrace);
        assert_eq!(s.grain, ConsolidationGrain::Session);
        assert_eq!(t.grain, ConsolidationGrain::Topic);
        assert_eq!(d.grain, ConsolidationGrain::DecisionTrace);
        assert_ne!(s.body, t.body);
        assert_ne!(t.body, d.body);
    }

    #[test]
    fn for_grain_pins_takeaways_count_per_proposal() {
        assert_eq!(
            Template::for_grain(ConsolidationGrain::Session).takeaways_count,
            3
        );
        assert_eq!(
            Template::for_grain(ConsolidationGrain::Topic).takeaways_count,
            5
        );
        assert_eq!(
            Template::for_grain(ConsolidationGrain::DecisionTrace).takeaways_count,
            7
        );
    }

    #[test]
    fn render_substitutes_known_slots_and_passes_through_unknown() {
        let tpl = Template::for_grain(ConsolidationGrain::Session);
        let rendered = tpl.render([("session_id", "01HXSESS01")]);
        assert!(
            rendered.contains("01HXSESS01"),
            "session_id slot must substitute"
        );
        assert!(
            !rendered.contains("{{session_id}}"),
            "substituted slot should not appear: {rendered}"
        );
        // An unknown slot stays literal so a misnamed key surfaces
        // in the prompt rather than silently emitting empty.
        assert!(
            rendered.contains("{{repo}}"),
            "untouched slot must stay literal: {rendered}"
        );
    }
}
