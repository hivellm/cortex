//! Cortex operator CLIs — consolidated.
//!
//! Single crate hosting the operator-facing CLIs:
//!
//! - [`bootstrap`] — bootstrap walker that republishes existing repo
//!   content as `cortex.events.bootstrap` (was crate
//!   `cortex-bootstrap`)
//! - [`ops`] — `cortex-ops` operator CLI: seed + cross-backend
//!   doctor (was crate `cortex-ops`)
//! - [`relevance_eval`] — `cortex-relevance-eval` recall@k / MRR
//!   harness (was crate `cortex-relevance-eval`, phase6e / F-008)
//!
//! Each module preserves the public API of its previous crate so
//! callers update imports with a one-line rewrite
//! (`cortex_bootstrap::Walker` → `cortex_cli::bootstrap::Walker`).

#![forbid(unsafe_code)]

pub mod bootstrap;
pub mod ops;
pub mod relevance_eval;
