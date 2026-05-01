//! Phase11i §3.6 — boot-time + SIGHUP relevance configuration.
//!
//! Bundles every operator-tunable knob the [`crate::fusion`]
//! module reads at fuse time:
//!
//! - RRF blend (`alpha`, `k`),
//! - cross-repo boost,
//! - session-cohesion boosts (same-session, cohort),
//! - outcome multipliers (success / error / blocked_by_law),
//! - per-intent recency-decay λ.
//!
//! Wire format is TOML so operators edit it the same way they
//! edit the daemon's other run-time configs. Compile-time defaults
//! mirror [`crate::fusion::FusionConfig::default`] + the per-intent
//! constants in `fusion.rs` so a missing file is the no-op (the
//! daemon stays at the same defaults a fresh checkout would have).
//!
//! The loader returns a [`RelevanceConfig`] which the boot path
//! folds into a [`FusionConfig`] via [`RelevanceConfig::base_fusion`].
//! SIGHUP-triggered reload calls
//! [`crate::orchestrator::Orchestrator::replace_fusion`] with a
//! fresh value built from the same TOML; the orchestrator's
//! `Arc<RwLock<FusionConfig>>` makes the swap visible to in-flight
//! requests on the next fuse call.

use std::path::Path;

use serde::Deserialize;

use crate::fusion::{
    FusionConfig, DEFAULT_COHORT_SESSION_BOOST, DEFAULT_CROSS_REPO_BOOST,
    DEFAULT_RECENCY_LAMBDA_DECISION, DEFAULT_RECENCY_LAMBDA_LAW, DEFAULT_RECENCY_LAMBDA_PRE_CHANGE,
    DEFAULT_RRF_ALPHA, DEFAULT_SAME_SESSION_BOOST, OUTCOME_MULT_BLOCKED_BY_LAW, OUTCOME_MULT_ERROR,
    OUTCOME_MULT_SUCCESS, RRF_K,
};

/// Loader error surface. Distinguishes "file unreadable" from
/// "file unparseable" so the boot path can keep going on the
/// former (defaults) but log loudly on the latter.
#[derive(Debug, thiserror::Error)]
pub enum RelevanceConfigError {
    /// I/O failure while reading the configured TOML path. Treated
    /// as "no override" by the boot path.
    #[error("read {path}: {source}")]
    Io {
        /// Path the daemon attempted to read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// TOML parse failure. The boot path falls back to defaults
    /// but logs a WARN so operators notice the typo.
    #[error("parse {path}: {source}")]
    Parse {
        /// Path the daemon attempted to parse.
        path: String,
        /// Underlying parse error.
        #[source]
        source: toml::de::Error,
    },
}

/// Phase11i §3.6 — fully resolved relevance knobs. Every field is
/// `Option`-less because [`RelevanceConfig::default`] supplies the
/// compile-time fallback for any missing key.
#[derive(Debug, Clone, PartialEq)]
pub struct RelevanceConfig {
    /// RRF blend weight in `[0.0, 1.0]`.
    pub alpha: f32,
    /// RRF stabilisation constant.
    pub k: u32,
    /// Cross-repo boost in `[0.0, 1.0]`.
    pub cross_repo_boost: f32,
    /// Multiplier for hits whose `extras["session_id"]` matches
    /// the active session.
    pub same_session_boost: f32,
    /// Multiplier for hits whose `extras["session_id"]` matches
    /// a sibling cohort entry.
    pub cohort_session_boost: f32,
    /// Outcome multipliers (success / error / blocked_by_law).
    pub outcome: OutcomeMultipliers,
    /// Per-intent recency-decay λ table.
    pub recency: RecencyTable,
}

/// Per-outcome multipliers applied after the allow / deny gate.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeMultipliers {
    /// Multiplier applied to admitted `outcome = "success"` hits.
    pub success: f32,
    /// Multiplier applied to admitted `outcome = "error"` hits.
    pub error: f32,
    /// Multiplier applied to admitted `outcome = "blocked_by_law"` hits.
    pub blocked_by_law: f32,
}

/// Per-intent recency-decay table.
#[derive(Debug, Clone, PartialEq)]
pub struct RecencyTable {
    /// λ for `pre_change_context`.
    pub pre_change_context: f32,
    /// λ for `similar_problems`.
    pub similar_problems: f32,
    /// λ for `free_search`.
    pub free_search: f32,
    /// λ for `explain`.
    pub explain: f32,
    /// λ for `decision_lookup`.
    pub decision_lookup: f32,
    /// λ for `law_check`.
    pub law_check: f32,
}

impl Default for RelevanceConfig {
    fn default() -> Self {
        Self {
            alpha: DEFAULT_RRF_ALPHA,
            k: RRF_K as u32,
            cross_repo_boost: DEFAULT_CROSS_REPO_BOOST,
            same_session_boost: DEFAULT_SAME_SESSION_BOOST,
            cohort_session_boost: DEFAULT_COHORT_SESSION_BOOST,
            outcome: OutcomeMultipliers::default(),
            recency: RecencyTable::default(),
        }
    }
}

impl Default for OutcomeMultipliers {
    fn default() -> Self {
        Self {
            success: OUTCOME_MULT_SUCCESS,
            error: OUTCOME_MULT_ERROR,
            blocked_by_law: OUTCOME_MULT_BLOCKED_BY_LAW,
        }
    }
}

impl Default for RecencyTable {
    fn default() -> Self {
        Self {
            pre_change_context: DEFAULT_RECENCY_LAMBDA_PRE_CHANGE,
            similar_problems: DEFAULT_RECENCY_LAMBDA_PRE_CHANGE,
            free_search: DEFAULT_RECENCY_LAMBDA_PRE_CHANGE,
            explain: DEFAULT_RECENCY_LAMBDA_PRE_CHANGE,
            decision_lookup: DEFAULT_RECENCY_LAMBDA_DECISION,
            law_check: DEFAULT_RECENCY_LAMBDA_LAW,
        }
    }
}

impl RelevanceConfig {
    /// Read + parse the TOML at `path`. A missing file surfaces as
    /// [`RelevanceConfigError::Io`] (the caller usually treats this
    /// as "no override"); a malformed file surfaces as
    /// [`RelevanceConfigError::Parse`] so operators see the typo.
    pub fn load_from_path(path: &Path) -> Result<Self, RelevanceConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|err| RelevanceConfigError::Io {
            path: path.display().to_string(),
            source: err,
        })?;
        Self::from_toml_str(&raw).map_err(|source| RelevanceConfigError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    /// Parse a TOML string into a fully resolved config. Missing
    /// keys fall back to compile-time defaults; out-of-range values
    /// are clamped (alpha to `[0.0, 1.0]`, cross-repo boost to
    /// `[0.0, 1.0]`, k to `>= 1`, every λ to `>= 0.0`).
    pub fn from_toml_str(raw: &str) -> Result<Self, toml::de::Error> {
        let parsed: RawRelevanceConfig = toml::from_str(raw)?;
        Ok(parsed.normalise())
    }

    /// Build a [`FusionConfig`] preloaded with this file's
    /// multiplier defaults. The orchestrator clones the result on
    /// boot and stamps it into its `Arc<RwLock<FusionConfig>>`.
    /// Per-request scope fields (recency_decay, cross_repo_boost,
    /// session_cohesion, outcome filter) still layer on top via
    /// the existing `with_*` builders during request handling.
    pub fn base_fusion(&self) -> FusionConfig {
        FusionConfig {
            alpha: self.alpha,
            k: self.k,
            recency_decay_lambda: 0.0, // per-request override
            now_ms: 0,
            cross_repo_boost: self.cross_repo_boost,
            active_repo: None,
            active_session_id: None,
            same_session_boost: self.same_session_boost,
            session_cohort: Vec::new(),
            cohort_session_boost: self.cohort_session_boost,
            outcomes_allow: Vec::new(),
            outcomes_deny: Vec::new(),
        }
    }

    /// Resolve the per-intent recency λ honouring this file's
    /// table. Centralised so the orchestrator can swap from the
    /// hard-coded `default_recency_lambda_for_intent` to the
    /// reloadable form without changing call-sites.
    pub fn recency_lambda_for_intent(&self, intent: &str) -> f32 {
        match intent {
            "pre_change_context" => self.recency.pre_change_context,
            "similar_problems" => self.recency.similar_problems,
            "free_search" => self.recency.free_search,
            "explain" => self.recency.explain,
            "decision_lookup" => self.recency.decision_lookup,
            "law_check" => self.recency.law_check,
            _ => 0.0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawRelevanceConfig {
    #[serde(default)]
    fusion: RawFusion,
    #[serde(default)]
    cross_repo: RawCrossRepo,
    #[serde(default)]
    session: RawSession,
    #[serde(default)]
    outcome: RawOutcome,
    #[serde(default)]
    recency: RawRecency,
}

#[derive(Debug, Default, Deserialize)]
struct RawFusion {
    alpha: Option<f32>,
    k: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCrossRepo {
    boost: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSession {
    same_session_boost: Option<f32>,
    cohort_session_boost: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOutcome {
    success: Option<f32>,
    error: Option<f32>,
    blocked_by_law: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRecency {
    pre_change_context: Option<f32>,
    similar_problems: Option<f32>,
    free_search: Option<f32>,
    explain: Option<f32>,
    decision_lookup: Option<f32>,
    law_check: Option<f32>,
}

impl RawRelevanceConfig {
    fn normalise(self) -> RelevanceConfig {
        let defaults = RelevanceConfig::default();
        RelevanceConfig {
            alpha: self
                .fusion
                .alpha
                .map(|v| v.clamp(0.0, 1.0))
                .unwrap_or(defaults.alpha),
            k: self.fusion.k.map(|v| v.max(1)).unwrap_or(defaults.k),
            cross_repo_boost: self
                .cross_repo
                .boost
                .map(|v| v.clamp(0.0, 1.0))
                .unwrap_or(defaults.cross_repo_boost),
            same_session_boost: self
                .session
                .same_session_boost
                .map(|v| v.max(0.0))
                .unwrap_or(defaults.same_session_boost),
            cohort_session_boost: self
                .session
                .cohort_session_boost
                .map(|v| v.max(0.0))
                .unwrap_or(defaults.cohort_session_boost),
            outcome: OutcomeMultipliers {
                success: self
                    .outcome
                    .success
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.outcome.success),
                error: self
                    .outcome
                    .error
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.outcome.error),
                blocked_by_law: self
                    .outcome
                    .blocked_by_law
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.outcome.blocked_by_law),
            },
            recency: RecencyTable {
                pre_change_context: self
                    .recency
                    .pre_change_context
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.recency.pre_change_context),
                similar_problems: self
                    .recency
                    .similar_problems
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.recency.similar_problems),
                free_search: self
                    .recency
                    .free_search
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.recency.free_search),
                explain: self
                    .recency
                    .explain
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.recency.explain),
                decision_lookup: self
                    .recency
                    .decision_lookup
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.recency.decision_lookup),
                law_check: self
                    .recency
                    .law_check
                    .map(|v| v.max(0.0))
                    .unwrap_or(defaults.recency.law_check),
            },
        }
    }
}

/// Resolve the path the daemon should read. Honours the
/// `CORTEX_RELEVANCE_CONFIG` env var (absolute or relative to
/// CWD); when unset, returns the in-tree default
/// `crates/cortex-api/config/relevance.toml` so the boot path
/// works in checkout-style deployments without further wiring.
pub fn default_config_path() -> std::path::PathBuf {
    if let Ok(raw) = std::env::var("CORTEX_RELEVANCE_CONFIG") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return std::path::PathBuf::from(trimmed);
        }
    }
    std::path::PathBuf::from("crates/cortex-api/config/relevance.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_compile_time_constants() {
        let cfg = RelevanceConfig::default();
        assert!((cfg.alpha - DEFAULT_RRF_ALPHA).abs() < f32::EPSILON);
        assert_eq!(cfg.k, RRF_K as u32);
        assert!((cfg.cross_repo_boost - DEFAULT_CROSS_REPO_BOOST).abs() < f32::EPSILON);
        assert!((cfg.same_session_boost - DEFAULT_SAME_SESSION_BOOST).abs() < f32::EPSILON);
        assert!((cfg.cohort_session_boost - DEFAULT_COHORT_SESSION_BOOST).abs() < f32::EPSILON);
        assert!((cfg.outcome.success - OUTCOME_MULT_SUCCESS).abs() < f32::EPSILON);
        assert!((cfg.outcome.error - OUTCOME_MULT_ERROR).abs() < f32::EPSILON);
        assert!((cfg.outcome.blocked_by_law - OUTCOME_MULT_BLOCKED_BY_LAW).abs() < f32::EPSILON);
        assert!(
            (cfg.recency.pre_change_context - DEFAULT_RECENCY_LAMBDA_PRE_CHANGE).abs()
                < f32::EPSILON
        );
        assert!(
            (cfg.recency.decision_lookup - DEFAULT_RECENCY_LAMBDA_DECISION).abs() < f32::EPSILON
        );
        assert!((cfg.recency.law_check - DEFAULT_RECENCY_LAMBDA_LAW).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_full_toml_payload() {
        let raw = r#"
            [fusion]
            alpha = 0.5
            k = 30
            [cross_repo]
            boost = 0.25
            [session]
            same_session_boost = 3.0
            cohort_session_boost = 1.75
            [outcome]
            success = 1.5
            error = 0.25
            blocked_by_law = 0.1
            [recency]
            pre_change_context = 0.04
            similar_problems = 0.04
            free_search = 0.04
            explain = 0.04
            decision_lookup = 0.01
            law_check = 0.0
        "#;
        let cfg = RelevanceConfig::from_toml_str(raw).expect("parse");
        assert!((cfg.alpha - 0.5).abs() < f32::EPSILON);
        assert_eq!(cfg.k, 30);
        assert!((cfg.cross_repo_boost - 0.25).abs() < f32::EPSILON);
        assert!((cfg.same_session_boost - 3.0).abs() < f32::EPSILON);
        assert!((cfg.cohort_session_boost - 1.75).abs() < f32::EPSILON);
        assert!((cfg.outcome.success - 1.5).abs() < f32::EPSILON);
        assert!((cfg.outcome.error - 0.25).abs() < f32::EPSILON);
        assert!((cfg.outcome.blocked_by_law - 0.1).abs() < f32::EPSILON);
        assert!((cfg.recency.pre_change_context - 0.04).abs() < f32::EPSILON);
        assert!((cfg.recency.decision_lookup - 0.01).abs() < f32::EPSILON);
        assert!((cfg.recency.law_check - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        // Empty TOML: every layer reverts to the compile-time
        // fallback. This is the "no override" guarantee.
        let cfg = RelevanceConfig::from_toml_str("").expect("parse");
        assert_eq!(cfg, RelevanceConfig::default());
    }

    #[test]
    fn out_of_range_values_clamp() {
        let raw = r#"
            [fusion]
            alpha = 2.5
            k = 0
            [cross_repo]
            boost = -0.4
            [session]
            same_session_boost = -1.0
            [outcome]
            error = -2.0
            [recency]
            pre_change_context = -0.5
        "#;
        let cfg = RelevanceConfig::from_toml_str(raw).expect("parse");
        assert!((cfg.alpha - 1.0).abs() < f32::EPSILON);
        assert_eq!(cfg.k, 1);
        assert!((cfg.cross_repo_boost - 0.0).abs() < f32::EPSILON);
        assert!((cfg.same_session_boost - 0.0).abs() < f32::EPSILON);
        assert!((cfg.outcome.error - 0.0).abs() < f32::EPSILON);
        assert!((cfg.recency.pre_change_context - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn malformed_toml_surfaces_parse_error() {
        let err = RelevanceConfig::from_toml_str("not = valid = toml").unwrap_err();
        assert!(format!("{err}").contains("expected"));
    }

    #[test]
    fn base_fusion_carries_file_defaults_only() {
        let cfg = RelevanceConfig::default();
        let fusion = cfg.base_fusion();
        // Per-request fields stay at "no-op" so the request-time
        // builders own the actual recency / session / outcome
        // gates.
        assert_eq!(fusion.recency_decay_lambda, 0.0);
        assert!(fusion.active_repo.is_none());
        assert!(fusion.active_session_id.is_none());
        assert!(fusion.outcomes_allow.is_empty());
        assert!(fusion.outcomes_deny.is_empty());
        // File-owned defaults reach the FusionConfig.
        assert!((fusion.alpha - DEFAULT_RRF_ALPHA).abs() < f32::EPSILON);
        assert_eq!(fusion.k, RRF_K as u32);
        assert!((fusion.cross_repo_boost - DEFAULT_CROSS_REPO_BOOST).abs() < f32::EPSILON);
        assert!((fusion.same_session_boost - DEFAULT_SAME_SESSION_BOOST).abs() < f32::EPSILON);
        assert!((fusion.cohort_session_boost - DEFAULT_COHORT_SESSION_BOOST).abs() < f32::EPSILON);
    }

    #[test]
    fn intent_table_resolves_known_and_unknown() {
        let cfg = RelevanceConfig::default();
        assert!(
            (cfg.recency_lambda_for_intent("pre_change_context")
                - DEFAULT_RECENCY_LAMBDA_PRE_CHANGE)
                .abs()
                < f32::EPSILON
        );
        assert!(
            (cfg.recency_lambda_for_intent("decision_lookup") - DEFAULT_RECENCY_LAMBDA_DECISION)
                .abs()
                < f32::EPSILON
        );
        assert_eq!(cfg.recency_lambda_for_intent("law_check"), 0.0);
        assert_eq!(cfg.recency_lambda_for_intent("unknown_intent"), 0.0);
    }

    #[test]
    fn default_config_path_honours_env_override() {
        // Save / restore the env so the test does not leak into
        // siblings that read CORTEX_RELEVANCE_CONFIG.
        let prior = std::env::var("CORTEX_RELEVANCE_CONFIG").ok();
        std::env::set_var("CORTEX_RELEVANCE_CONFIG", "/tmp/relevance.toml");
        let got = default_config_path();
        assert_eq!(got, std::path::PathBuf::from("/tmp/relevance.toml"));
        std::env::set_var("CORTEX_RELEVANCE_CONFIG", "");
        let got = default_config_path();
        assert_eq!(
            got,
            std::path::PathBuf::from("crates/cortex-api/config/relevance.toml")
        );
        match prior {
            Some(v) => std::env::set_var("CORTEX_RELEVANCE_CONFIG", v),
            None => std::env::remove_var("CORTEX_RELEVANCE_CONFIG"),
        }
    }
}
