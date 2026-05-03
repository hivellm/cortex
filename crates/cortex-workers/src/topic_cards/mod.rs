//! Phase11r §2 — living-synthesis topic cards on top of consolidations.
//!
//! Where `cortex_workers::consolidator` ships retrospective summaries
//! (one Session / Topic / DecisionTrace consolidation per source set),
//! `cortex_workers::topic_cards` ships the **reactive** living surface
//! the model consults via MCP. The LLM rewrites a card's
//! `synthesis_markdown` whenever new evidence lands; contradictions
//! surface explicitly; staleness signals (`synthesis_age_d`,
//! `events_since_last_rev`) drive renderer downgrade.
//!
//! Module map:
//!
//! - [`templates`] — the `rewrite.md` Markdown template (loaded via
//!   `include_str!`) plus the `RewriteSlots` struct + render fn.
//! - [`synthesiser`] — wraps the existing
//!   [`crate::consolidator::summariser::Summariser`] trait, parses the
//!   model's JSON output into [`synthesiser::RewriteOutput`]. Composition
//!   over duplication — no new abstract trait.
//! - [`producer`] — the rewrite pipeline that builds a
//!   [`cortex_core::events::TopicCardPayload`] from a
//!   [`producer::ProduceInput`] + [`synthesiser::TopicCardSynthesiser`].
//!   Stamps `revision`, dedupes evidence, validates cross-field rules.
//! - [`trigger`] — reactive heuristic that decides whether an inbound
//!   event should fire a rewrite or be held. Three signals:
//!   `events_since_last_rev >= 8`, `embedding_distance < 0.30 AND
//!   high-impact`, or `synthesis_age_d >= 14 AND any new evidence`.
//! - [`contradictions`] — heuristic detector with three classes:
//!   `decision_supersession`, `law_violation_mismatch`,
//!   `outcome_divergence`.
//! - [`orchestrator`] — cost-budget-aware dispatcher. Reuses the
//!   [`crate::consolidator::cost_telemetry::CostBudget`] +
//!   [`crate::consolidator::cost_telemetry::CostLedger`] under the
//!   `topic_card` grain bucket. Auto-promotes Haiku → Opus on
//!   contradiction-count threshold or operator override.
//! - [`prelude`] — single-import surface for downstream callers.
//!
//! Per ADR-007, this lives as a module under `cortex-workers` rather
//! than as a new top-level crate. The original phase11r §2.1 plan
//! called for `cortex-topic-cards`; the merge contract from phase11s
//! supersedes that.

#![warn(missing_docs)]

pub mod contradictions;
pub mod orchestrator;
pub mod producer;
pub mod synthesiser;
pub mod templates;
pub mod trigger;

pub mod prelude;
