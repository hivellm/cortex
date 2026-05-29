//! phase18 §2.8 — Versioned reindex aliases for the bitemporal cutover.
//!
//! The migration in §2.9 (`cortex-ops migrate-bitemporal`) walks
//! existing rows and writes the bitemporal-stamped copies into
//! versioned Meili + Vectorizer aliases. When the operator verifies
//! the bitemporal slice, the reindex job flips the read pointer to
//! the new alias. A rollback is `alias --revert` — no data is
//! destroyed.
//!
//! The convention coexists with the existing per-repo
//! `cortex-<slug>-<family>` indexes — the bitemporal reindex copies
//! the rows into the v1 alias; the temporal classifier (§3) reads
//! from the alias when enabled.

/// Phase18 §2.8 — current Meili reindex alias (`v1`). When the
/// classifier hits a row written before §2.6 / §2.7 landed (no
/// `project_id`, no `valid_from_unix`), the alias indirection lets
/// the read path fall back to the original
/// `cortex-<slug>-<family>` index without an alias rename.
pub const MEILI_BITEMPORAL_ALIAS: &str = "cortex-meili-bitemporal-v1";

/// Phase18 §2.8 — current Vectorizer reindex alias (`v1`).
pub const VECTORIZER_BITEMPORAL_ALIAS: &str = "cortex-vector-bitemporal-v1";

/// Phase18 §2.8 — every alias the migration may need to flip /
/// revert. Operators reference this slice in the `cortex-ops
/// alias` subcommand (§2.9).
pub const ALL_BITEMPORAL_ALIASES: &[&str] = &[
    MEILI_BITEMPORAL_ALIAS,
    VECTORIZER_BITEMPORAL_ALIAS,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_share_v1_suffix_for_rollback_pairing() {
        for alias in ALL_BITEMPORAL_ALIASES {
            assert!(
                alias.ends_with("-v1"),
                "alias `{alias}` must carry the `-v1` suffix so a future v2 cutover stays pairwise revertible"
            );
        }
    }

    #[test]
    fn aliases_use_cortex_prefix() {
        for alias in ALL_BITEMPORAL_ALIASES {
            assert!(
                alias.starts_with("cortex-"),
                "alias `{alias}` must carry the `cortex-` prefix; bare names collide with non-cortex tenants on shared infrastructure"
            );
        }
    }

    #[test]
    fn alias_set_covers_meili_and_vectorizer() {
        assert!(ALL_BITEMPORAL_ALIASES.contains(&MEILI_BITEMPORAL_ALIAS));
        assert!(ALL_BITEMPORAL_ALIASES.contains(&VECTORIZER_BITEMPORAL_ALIAS));
        assert_eq!(ALL_BITEMPORAL_ALIASES.len(), 2);
    }
}
