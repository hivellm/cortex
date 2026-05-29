//! Phase18 §3.1-§3.2 — Temporal classifier state machine.
//!
//! State priority (first match wins; the comparison runs in order
//! and never re-enters):
//!
//! 1. `SUPERSEDED` — `superseded_at IS NOT NULL AND superseded_at <= as_of`
//! 2. `EXPIRED`    — `valid_to IS NOT NULL AND valid_to <= as_of`
//! 3. `NOT_YET_VALID` — `valid_from > as_of`
//! 4. `ABANDONED`  — `lifecycle == "abandoned"`
//! 5. `TEMPORAL`   — `valid_to IS NOT NULL AND valid_to <= as_of + threshold`
//! 6. `VALID`      — default
//!
//! Per-state action depends on the caller-supplied `IncludeFlags`
//! (default: drop everything but VALID + TEMPORAL). The TEMPORAL
//! state's recency-boost multiplier is operator-tuned via
//! `TemporalConfig::temporal_boost` (default 1.10 per design.md
//! §2.2).
//!
//! Inputs use epoch-second integers so the classifier composes
//! with the Meili `*_unix` filterable axes (phase18 §2.6) without
//! re-parsing RFC3339 strings on the hot path. Conversion happens
//! at the orchestrator boundary (`crates/cortex-api/src/search/orchestrator.rs`
//! lands the wiring in §3.3 + §3.5).

use serde::{Deserialize, Serialize};

/// Per-candidate state per design.md §2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalState {
    /// Fact is current and stable.
    Valid,
    /// Fact is active but `valid_to` lands inside the recency
    /// window — apply the recency boost.
    Temporal,
    /// Fact was replaced by a newer fact.
    Superseded,
    /// Fact's `valid_to` already elapsed at the requested `as_of`.
    Expired,
    /// Fact's `valid_from` lands in the future relative to `as_of`.
    NotYetValid,
    /// Fact lives on an abandoned branch.
    Abandoned,
}

impl TemporalState {
    /// Stable lowercase label for telemetry + JSON audit envelopes.
    pub fn as_str(self) -> &'static str {
        match self {
            TemporalState::Valid => "valid",
            TemporalState::Temporal => "temporal",
            TemporalState::Superseded => "superseded",
            TemporalState::Expired => "expired",
            TemporalState::NotYetValid => "not_yet_valid",
            TemporalState::Abandoned => "abandoned",
        }
    }
}

/// Action the classifier emits per candidate. Drives the
/// orchestrator's score adjustment after fusion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Pass through with `score = base * multiplier` (1.0 = no change).
    Pass { multiplier: f32 },
    /// Drop the hit from the candidate set entirely.
    Drop,
    /// Demote — keep the hit but multiply score by the demote
    /// factor. Used when `IncludeFlags` keeps a historical fact in
    /// the bundle for audit but does not want it competing for the
    /// top-K.
    Demote { factor: f32 },
}

/// Inputs the classifier reads off each candidate. Kept narrow so
/// the orchestrator builds it once per `LaneHit` without
/// re-walking the doc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    /// `valid_from_unix` from the Meili / Vectorizer payload.
    /// `None` when the column was absent (pre-migration row).
    pub valid_from_unix: Option<i64>,
    /// `valid_to_unix`; `None` = still valid.
    pub valid_to_unix: Option<i64>,
    /// `superseded_at_unix`; `None` = still believed.
    pub superseded_at_unix: Option<i64>,
    /// `lifecycle` label. `None` = no lifecycle stamp (treated as
    /// `"active"` for backwards compat with pre-migration rows).
    pub lifecycle: Option<&'static str>,
}

/// Operator flags that flip the default `Drop` decisions into
/// keep-or-demote. Default is "current scope only" — every
/// historical / future / abandoned candidate drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IncludeFlags {
    /// When set, SUPERSEDED + EXPIRED candidates get demoted
    /// instead of dropped.
    pub include_history: bool,
    /// When set, NOT_YET_VALID candidates get demoted instead of
    /// dropped — used by planning queries.
    pub include_future: bool,
    /// When set, ABANDONED candidates get demoted instead of
    /// dropped — used when the caller scopes to a specific branch
    /// that includes the abandoned line.
    pub include_branches: bool,
}

/// Tunable config for the classifier. Defaults match design.md
/// §2.2 (recency boost 1.10, recency window 30 days). The orchestrator
/// reads it off `cortex-config::TemporalConfig` (§3.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemporalConfig {
    /// Recency boost multiplier for the TEMPORAL state. 1.10 = +10%.
    pub temporal_boost: f32,
    /// Recency window in seconds — a fact whose `valid_to` lands
    /// within `as_of + window` enters the TEMPORAL state. 30 days
    /// default.
    pub temporal_window_seconds: i64,
    /// Demote factor for `include_*` keeps. 0.5 = halve the score.
    pub demote_factor: f32,
}

impl Default for TemporalConfig {
    fn default() -> Self {
        Self {
            temporal_boost: 1.10,
            temporal_window_seconds: 30 * 24 * 60 * 60,
            demote_factor: 0.5,
        }
    }
}

/// Pure classifier — run the state machine + apply the operator
/// flags + the config tuning to produce a single `Action`. Side-
/// effect free; the orchestrator wraps it with audit logging
/// (§3.9) at the call site.
pub fn classify(
    candidate: &Candidate,
    as_of_unix: i64,
    flags: IncludeFlags,
    config: &TemporalConfig,
) -> (TemporalState, Action) {
    let state = derive_state(candidate, as_of_unix, config.temporal_window_seconds);
    let action = match state {
        TemporalState::Valid => Action::Pass { multiplier: 1.0 },
        TemporalState::Temporal => Action::Pass {
            multiplier: config.temporal_boost,
        },
        TemporalState::Superseded | TemporalState::Expired => {
            if flags.include_history {
                Action::Demote {
                    factor: config.demote_factor,
                }
            } else {
                Action::Drop
            }
        }
        TemporalState::NotYetValid => {
            if flags.include_future {
                Action::Demote {
                    factor: config.demote_factor,
                }
            } else {
                Action::Drop
            }
        }
        TemporalState::Abandoned => {
            if flags.include_branches {
                Action::Demote {
                    factor: config.demote_factor,
                }
            } else {
                Action::Drop
            }
        }
    };
    (state, action)
}

fn derive_state(c: &Candidate, as_of: i64, window: i64) -> TemporalState {
    // 1. SUPERSEDED beats every other lifecycle signal — once
    // Cortex stops believing the fact, no temporal axis re-promotes
    // it.
    if let Some(s) = c.superseded_at_unix {
        if s <= as_of {
            return TemporalState::Superseded;
        }
    }
    // 2. EXPIRED — `valid_to` elapsed.
    if let Some(v) = c.valid_to_unix {
        if v <= as_of {
            return TemporalState::Expired;
        }
    }
    // 3. NOT_YET_VALID — `valid_from` lands in the future.
    if let Some(v) = c.valid_from_unix {
        if v > as_of {
            return TemporalState::NotYetValid;
        }
    }
    // 4. ABANDONED — lifecycle stamp wins over the temporal
    // window check (an abandoned fact in the TEMPORAL window is
    // still abandoned for retrieval purposes).
    if matches!(c.lifecycle, Some("abandoned")) {
        return TemporalState::Abandoned;
    }
    // 5. TEMPORAL — `valid_to` lands inside the recency window.
    if let Some(v) = c.valid_to_unix {
        if v <= as_of.saturating_add(window) {
            return TemporalState::Temporal;
        }
    }
    // 6. VALID — default.
    TemporalState::Valid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TemporalConfig {
        TemporalConfig::default()
    }

    fn base() -> Candidate {
        Candidate {
            valid_from_unix: Some(0),
            valid_to_unix: None,
            superseded_at_unix: None,
            lifecycle: Some("active"),
        }
    }

    #[test]
    fn default_lane_yields_valid_with_unit_multiplier() {
        let (state, action) = classify(&base(), 1_700_000_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::Valid);
        assert!(matches!(action, Action::Pass { multiplier } if (multiplier - 1.0).abs() < 1e-6));
    }

    #[test]
    fn superseded_drops_by_default_and_demotes_with_include_history() {
        let c = Candidate {
            superseded_at_unix: Some(1_000),
            ..base()
        };
        let (state, action) = classify(&c, 2_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::Superseded);
        assert!(matches!(action, Action::Drop));

        let flags = IncludeFlags {
            include_history: true,
            ..Default::default()
        };
        let (_, action) = classify(&c, 2_000, flags, &cfg());
        assert!(matches!(action, Action::Demote { factor } if (factor - 0.5).abs() < 1e-6));
    }

    #[test]
    fn expired_takes_precedence_over_temporal_window() {
        let c = Candidate {
            valid_to_unix: Some(1_000),
            ..base()
        };
        let (state, _) = classify(&c, 2_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::Expired);
    }

    #[test]
    fn not_yet_valid_fires_when_valid_from_is_in_the_future() {
        let c = Candidate {
            valid_from_unix: Some(5_000),
            ..base()
        };
        let (state, action) = classify(&c, 2_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::NotYetValid);
        assert!(matches!(action, Action::Drop));

        let flags = IncludeFlags {
            include_future: true,
            ..Default::default()
        };
        let (_, action) = classify(&c, 2_000, flags, &cfg());
        assert!(matches!(action, Action::Demote { .. }));
    }

    #[test]
    fn abandoned_lifecycle_drops_by_default_and_demotes_with_include_branches() {
        let c = Candidate {
            lifecycle: Some("abandoned"),
            ..base()
        };
        let (state, action) = classify(&c, 2_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::Abandoned);
        assert!(matches!(action, Action::Drop));

        let flags = IncludeFlags {
            include_branches: true,
            ..Default::default()
        };
        let (_, action) = classify(&c, 2_000, flags, &cfg());
        assert!(matches!(action, Action::Demote { .. }));
    }

    #[test]
    fn temporal_state_applies_recency_boost() {
        // valid_to lands 5 days from now, within the default 30-day window.
        let c = Candidate {
            valid_to_unix: Some(2_000 + 5 * 24 * 60 * 60),
            ..base()
        };
        let (state, action) = classify(&c, 2_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::Temporal);
        assert!(matches!(action, Action::Pass { multiplier } if (multiplier - 1.10).abs() < 1e-6));
    }

    #[test]
    fn temporal_state_skipped_when_valid_to_is_past_window() {
        // valid_to lands 60 days from now — past the 30-day window.
        let c = Candidate {
            valid_to_unix: Some(2_000 + 60 * 24 * 60 * 60),
            ..base()
        };
        let (state, _) = classify(&c, 2_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::Valid);
    }

    #[test]
    fn state_priority_superseded_wins_over_expired_and_abandoned() {
        let c = Candidate {
            valid_to_unix: Some(500),
            superseded_at_unix: Some(1_000),
            lifecycle: Some("abandoned"),
            ..base()
        };
        let (state, _) = classify(&c, 2_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::Superseded);
    }

    #[test]
    fn missing_columns_default_to_valid() {
        let c = Candidate {
            valid_from_unix: None,
            valid_to_unix: None,
            superseded_at_unix: None,
            lifecycle: None,
        };
        let (state, _) = classify(&c, 2_000, IncludeFlags::default(), &cfg());
        assert_eq!(state, TemporalState::Valid);
    }

    #[test]
    fn temporal_config_default_matches_design_doc() {
        let cfg = TemporalConfig::default();
        assert!((cfg.temporal_boost - 1.10).abs() < 1e-6);
        assert_eq!(cfg.temporal_window_seconds, 30 * 24 * 60 * 60);
        assert!((cfg.demote_factor - 0.5).abs() < 1e-6);
    }

    #[test]
    fn temporal_state_as_str_round_trips() {
        for s in [
            TemporalState::Valid,
            TemporalState::Temporal,
            TemporalState::Superseded,
            TemporalState::Expired,
            TemporalState::NotYetValid,
            TemporalState::Abandoned,
        ] {
            assert!(!s.as_str().is_empty());
            assert!(s
                .as_str()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }
}
