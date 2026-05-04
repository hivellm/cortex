//! Loader for the versioned settings.v1.json file.
//!
//! The settings file lives at `crates/cortex-fulltext/settings/`. The
//! crate ships its v1 contents inline via `include_str!` so the
//! binary doesn't need disk access to bootstrap; `load_settings_v1`
//! provides an alternate path-based loader for tests and for the
//! `cortex-fulltext-worker` binary when an operator wants to override
//! the in-tree version.

use std::fs;
use std::path::Path;

use serde_json::Value;
use thiserror::Error;

/// Source for the v1 settings, baked into the binary at build time.
pub const SETTINGS_V1: &str = include_str!("../../settings/settings.v1.json");

/// Failure modes while loading settings from disk.
#[derive(Debug, Error)]
pub enum SettingsLoadError {
    /// Filesystem access failed.
    #[error("settings file unreadable: {0}")]
    Io(#[from] std::io::Error),
    /// JSON parse failed.
    #[error("settings json parse failed: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Parse the baked-in v1 settings.
pub fn settings_v1_json() -> Result<Value, SettingsLoadError> {
    Ok(serde_json::from_str(SETTINGS_V1)?)
}

/// Load settings from a file on disk (operator-supplied override).
pub fn load_settings_v1(path: &Path) -> Result<Value, SettingsLoadError> {
    let body = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_in_settings_parse() {
        let v = settings_v1_json().expect("v1 settings parse");
        // Phase11r §3.3 — bumped to v6 so the settings-version
        // watcher triggers a re-index pass that adds the
        // `ext.topic_card.*` axis fields the topic-card lane needs
        // for filtering / sorting / search. Phase11k's v5 added the
        // governance top-level projection (`decision_id` / `law_id` /
        // `turn_id` …); phase11j's v4 added `ext.consolidation.*`;
        // phase11i's v3 added `model` / `tool` / `session_id` /
        // `outcome`; phase11b's v2 introduced `path_prefixes`.
        assert_eq!(v["version"], "v6");
        assert!(v["searchableAttributes"].as_array().unwrap().len() >= 4);
        assert!(v["sortableAttributes"]
            .as_array()
            .unwrap()
            .contains(&Value::String("ts".to_string())));
    }

    #[test]
    fn consolidation_axis_fields_are_filterable_and_sortable() {
        // Phase11j §3.4 — the dashboard memory view filters the
        // consolidations lane on grain / depth / model and sorts on
        // consolidation_id. Pin the contract so a future settings
        // rev cannot drop one without surfacing here first.
        let v = settings_v1_json().expect("v1 settings parse");
        let filterable = v["filterableAttributes"]
            .as_array()
            .expect("filterableAttributes is an array");
        let sortable = v["sortableAttributes"]
            .as_array()
            .expect("sortableAttributes is an array");
        let searchable = v["searchableAttributes"]
            .as_array()
            .expect("searchableAttributes is an array");
        for required in [
            "ext.consolidation.grain",
            "ext.consolidation.depth",
            "ext.consolidation.model",
            "ext.consolidation.consolidation_id",
        ] {
            assert!(
                filterable.contains(&Value::String(required.to_string())),
                "filterableAttributes must include `{required}` for phase11j §3.4",
            );
            assert!(
                sortable.contains(&Value::String(required.to_string())),
                "sortableAttributes must include `{required}` for phase11j §3.4",
            );
        }
        assert!(
            searchable.contains(&Value::String(
                "ext.consolidation.outcome_distribution".to_string()
            )),
            "searchableAttributes must include `ext.consolidation.outcome_distribution` for phase11j §3.4",
        );
    }

    #[test]
    fn topic_card_axis_fields_are_filterable_and_sortable() {
        // Phase11r §3.3 — the dashboard memory view (and the
        // pre-thinking lane) filters the topic-cards lane on
        // topic_slug / revision / confidence / synthesis_age_d /
        // contradictions_count / repos / synthesis_model and sorts
        // on revision / confidence / synthesis_age_d /
        // contradictions_count. `synthesis_markdown` + `topic_slug`
        // are searchable so a free-text query against the
        // synthesis body still hits the row.
        let v = settings_v1_json().expect("v1 settings parse");
        let filterable = v["filterableAttributes"]
            .as_array()
            .expect("filterableAttributes is an array");
        let sortable = v["sortableAttributes"]
            .as_array()
            .expect("sortableAttributes is an array");
        let searchable = v["searchableAttributes"]
            .as_array()
            .expect("searchableAttributes is an array");
        for required in [
            "ext.topic_card.topic_slug",
            "ext.topic_card.revision",
            "ext.topic_card.confidence",
            "ext.topic_card.synthesis_age_d",
            "ext.topic_card.contradictions_count",
            "ext.topic_card.repos",
            "ext.topic_card.synthesis_model",
        ] {
            assert!(
                filterable.contains(&Value::String(required.to_string())),
                "filterableAttributes must include `{required}` for phase11r §3.3",
            );
        }
        for required in [
            "ext.topic_card.topic_slug",
            "ext.topic_card.revision",
            "ext.topic_card.confidence",
            "ext.topic_card.synthesis_age_d",
            "ext.topic_card.contradictions_count",
            "ext.topic_card.synthesis_model",
        ] {
            assert!(
                sortable.contains(&Value::String(required.to_string())),
                "sortableAttributes must include `{required}` for phase11r §3.3",
            );
        }
        for required in [
            "ext.topic_card.synthesis_markdown",
            "ext.topic_card.topic_slug",
        ] {
            assert!(
                searchable.contains(&Value::String(required.to_string())),
                "searchableAttributes must include `{required}` for phase11r §3.3",
            );
        }
    }

    #[test]
    fn relevance_axis_fields_are_filterable() {
        // Phase11i §3.3-§3.5 — every axis-driving field must
        // appear in filterableAttributes so the keyword lane can
        // filter on it. The relevance-tuning IT depends on this
        // contract; pin it explicitly so a future settings rev
        // cannot silently drop one.
        let v = settings_v1_json().expect("v1 settings parse");
        let filterable = v["filterableAttributes"]
            .as_array()
            .expect("filterableAttributes is an array");
        for required in ["model", "tool", "session_id", "outcome"] {
            assert!(
                filterable.contains(&Value::String(required.to_string())),
                "filterableAttributes must include `{required}` for phase11i §3.3-§3.5",
            );
        }
    }

    #[test]
    fn governance_top_level_fields_are_filterable_and_searchable() {
        // Phase11k §1.3 — the spec-11 lane projection contract pulls
        // these keys out of `extras_raw` for the read-side overlay
        // derivers. They MUST be filterable so callers can scope
        // (`decision_status = 'accepted'`, `law_severity =
        // 'critical'`); the human-facing ones (`decision_title`,
        // `law_id`) MUST be searchable so a free-text query against
        // an ADR title still hits the row.
        let v = settings_v1_json().expect("v1 settings parse");
        let filterable = v["filterableAttributes"]
            .as_array()
            .expect("filterableAttributes is an array");
        let searchable = v["searchableAttributes"]
            .as_array()
            .expect("searchableAttributes is an array");
        for required in [
            "decision_id",
            "decision_status",
            "decision_supersedes",
            "law_id",
            "law_severity",
            "law_tier",
            "turn_id",
        ] {
            assert!(
                filterable.contains(&Value::String(required.to_string())),
                "filterableAttributes must include `{required}` for phase11k §1.3",
            );
        }
        for required in ["decision_title", "law_id"] {
            assert!(
                searchable.contains(&Value::String(required.to_string())),
                "searchableAttributes must include `{required}` for phase11k §1.3",
            );
        }
    }

    #[test]
    fn path_prefixes_is_filterable() {
        // Phase11b §2.2 — `meili_lane::build_filter` emits
        // `path_prefixes IN [...]`; Meilisearch rejects filters on
        // attributes that aren't declared filterable, so this is the
        // load-bearing assertion.
        let v = settings_v1_json().expect("v1 settings parse");
        let filterable = v["filterableAttributes"]
            .as_array()
            .expect("filterableAttributes is an array");
        assert!(
            filterable.contains(&Value::String("path_prefixes".to_string())),
            "filterableAttributes must include `path_prefixes`",
        );
    }
}
