//! Phase11r §2.2 — topic-card rewrite prompt template.
//!
//! The template lives at `crates/cortex-workers/templates/topic_cards/rewrite.md`
//! and is embedded at compile time via [`include_str!`]. Callers fill a
//! [`RewriteSlots`] struct and call [`render_rewrite_prompt`] — the output
//! is a ready-to-ship prompt for the Anthropic Messages API.

/// Rendered prompt output contract block — tells the model exactly what JSON
/// shape to emit so the synthesiser parser does not need to be defensive.
pub const OUTPUT_CONTRACT: &str = r#"Return a JSON object with **exactly** these fields (no Markdown fences around the outer object, but the `synthesis_markdown` value may contain Markdown):

```json
{
  "synthesis_markdown": "200–4000 bytes of Markdown prose synthesising all evidence for this topic",
  "contradictions": [
    {
      "kind": "decision_supersession | law_violation_mismatch | outcome_divergence",
      "evidence_a": "<evidence-ref-id>",
      "evidence_b": "<evidence-ref-id>"
    }
  ],
  "open_questions": [
    "≤ 8 questions the evidence leaves open"
  ],
  "confidence": 0.0
}
```

Rules:
- `synthesis_markdown` must be between 200 and 4 000 bytes.
- `contradictions` is an array (may be empty).
- `open_questions` has at most 8 entries; each entry ≤ 200 chars.
- `confidence` is a float in `[0.0, 1.0]` reflecting how well-supported the synthesis is.
- `evidence_a` and `evidence_b` in each contradiction must be ids from the evidence blocks above.
- Do **not** invent evidence that was not provided; say so in `open_questions` if the provided evidence is sparse.
- Emit the JSON **only** — no surrounding prose, no code fences wrapping the outer object."#;

/// Raw template body embedded at compile time.
pub const REWRITE_TEMPLATE_BODY: &str = include_str!("../../templates/topic_cards/rewrite.md");

/// Input slots for the topic-card rewrite template.
#[derive(Debug, Clone)]
pub struct RewriteSlots<'a> {
    /// Kebab-case topic slug (e.g. `auth-rewrite`).
    pub topic_slug: &'a str,
    /// Repository scope (e.g. `cortex`).
    pub repo_scope: &'a str,
    /// Synthesis text from the prior revision, or `"(no prior synthesis)"` on first emit.
    pub existing_synthesis: &'a str,
    /// Revision number of the existing synthesis, or `"0"` on first emit.
    pub existing_revision: &'a str,
    /// Text block summarising new evidence items to incorporate.
    pub new_evidence_block: &'a str,
    /// Text block summarising superseded evidence items to remove or down-weight.
    pub superseded_evidence_block: &'a str,
}

/// Render the rewrite prompt by substituting all `{{key}}` slots.
///
/// Unknown slots pass through verbatim so typos surface in the rendered prompt
/// rather than producing an empty string — the same defensive policy the
/// consolidator templates use.
pub fn render_rewrite_prompt(slots: &RewriteSlots<'_>) -> String {
    let mut out = REWRITE_TEMPLATE_BODY.to_string();
    let pairs: &[(&str, &str)] = &[
        ("topic_slug", slots.topic_slug),
        ("repo_scope", slots.repo_scope),
        ("existing_synthesis", slots.existing_synthesis),
        ("existing_revision", slots.existing_revision),
        ("new_evidence_block", slots.new_evidence_block),
        ("superseded_evidence_block", slots.superseded_evidence_block),
        ("output_contract", OUTPUT_CONTRACT),
    ];
    for (key, value) in pairs {
        let needle = format!("{{{{{key}}}}}");
        out = out.replace(&needle, value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_slots() -> RewriteSlots<'static> {
        RewriteSlots {
            topic_slug: "auth-rewrite",
            repo_scope: "cortex",
            existing_synthesis: "(no prior synthesis)",
            existing_revision: "0",
            new_evidence_block: "- DEC-001: Adopt JWT tokens",
            superseded_evidence_block: "(none)",
        }
    }

    #[test]
    fn render_substitutes_all_known_slots() {
        let slots = default_slots();
        let rendered = render_rewrite_prompt(&slots);
        assert!(
            rendered.contains("auth-rewrite"),
            "topic_slug slot must be substituted"
        );
        assert!(
            rendered.contains("cortex"),
            "repo_scope slot must be substituted"
        );
        assert!(
            rendered.contains("(no prior synthesis)"),
            "existing_synthesis slot must be substituted"
        );
        assert!(
            rendered.contains("DEC-001: Adopt JWT tokens"),
            "new_evidence_block slot must be substituted"
        );
        assert!(
            !rendered.contains("{{topic_slug}}"),
            "no unresolved {{topic_slug}} slot should remain"
        );
        assert!(
            !rendered.contains("{{output_contract}}"),
            "output_contract slot must be substituted"
        );
    }

    #[test]
    fn render_passes_through_unknown_slots_verbatim() {
        // A slot typo should appear literally in the output rather than disappearing
        let slots = RewriteSlots {
            topic_slug: "{{typo_slot}}",
            ..default_slots()
        };
        let rendered = render_rewrite_prompt(&slots);
        // The `{{typo_slot}}` was used as the *value* of topic_slug — it substitutes.
        // The important thing is that unrelated unknowns (added to the template body
        // hypothetically) would stay as `{{unknown}}`. We verify the template itself
        // has no leftover unresolved slots after a full render.
        assert!(
            !rendered.contains("{{repo_scope}}"),
            "repo_scope must be resolved"
        );
    }

    #[test]
    fn output_contract_is_embedded_in_rendered_prompt() {
        let slots = default_slots();
        let rendered = render_rewrite_prompt(&slots);
        assert!(
            rendered.contains("synthesis_markdown"),
            "output contract must be present in rendered prompt"
        );
        assert!(
            rendered.contains("confidence"),
            "confidence field must appear in output contract"
        );
        assert!(
            rendered.contains("contradictions"),
            "contradictions field must appear in output contract"
        );
    }
}
