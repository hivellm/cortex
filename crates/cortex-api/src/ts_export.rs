//! Phase14d — central ts-rs export driver.
//!
//! Gated behind the `ts-export` feature so the production build
//! does not pull `ts-rs` and the export test does not fire during
//! the normal `cargo test --workspace` cycle. Invoked by
//! `scripts/generate-gui-types.sh` to refresh
//! `gui/src/lib/api.generated.ts`.
//!
//! ## Adding a new wire type to the bundle
//!
//! 1. On the type's declaration, add:
//!    ```rust,ignore
//!    #[cfg_attr(feature = "ts-export", derive(ts_rs::TS))]
//!    #[cfg_attr(
//!        feature = "ts-export",
//!        ts(export, export_to = "../../gui-types/")
//!    )]
//!    ```
//! 2. Add the type to the [`export_all_wire_types`] list below.
//! 3. Re-run `bash scripts/generate-gui-types.sh` and commit the
//!    diff. CI's `pnpm -C gui run check-contract` will fail any
//!    PR whose bundle disagrees with the regenerated output.

#![cfg(feature = "ts-export")]

use ts_rs::TS;

// `dashboard::consolidations` is `mod` (not pub) so re-export
// the wire type through the `dashboard` facade. The bundle uses
// the simpler symbol path.
use crate::dashboard::ConsolidationFilter;
use crate::health::consolidator::{ConsolidatorHealthReport, GrainHealth};
use crate::health::pre_thinking::{
    IntentByteQuantilesView, IntentHelpfulRateView, IntentMismatchView, PreThinkingHealthReport,
};
use crate::health::{FreshnessRow, Severity};

/// Drive `ts-rs` for every wire type registered with the bundle.
/// Single canonical entrypoint so `scripts/generate-gui-types.sh`
/// only has to run one test to refresh the GUI bundle.
pub fn export_all_wire_types() -> Result<(), ts_rs::ExportError> {
    GrainHealth::export_all()?;
    ConsolidatorHealthReport::export_all()?;
    Severity::export_all()?;
    FreshnessRow::export_all()?;
    ConsolidationFilter::export_all()?;
    PreThinkingHealthReport::export_all()?;
    IntentByteQuantilesView::export_all()?;
    IntentHelpfulRateView::export_all()?;
    IntentMismatchView::export_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_export_emits_every_registered_wire_type() {
        export_all_wire_types().expect("export every registered type");
    }
}
