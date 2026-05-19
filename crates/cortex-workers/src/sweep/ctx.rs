//! [`SweepCtx`] — the shared environment passed to every
//! [`Sweep::run`](super::Sweep::run) invocation.
//!
//! Design note (ADR-009 §2.2). The proposal text reads "SweepCtx
//! carries handles to MetadataStore, Vectorizer, Meili, Nexus, the
//! worker config, and a logger". Realised pragmatically: the ctx
//! carries only what is **shared across every sweep**:
//!
//! - the metadata-store handle (every sweep writes a
//!   `retention_sweeps` row; the SQLite connection is the one piece
//!   of infrastructure no sweep can skip),
//! - the reference clock (`now`) so sweeps are time-travellable
//!   under test without a global mock,
//! - the shared `SweepConfig` (batch sizes, error-rate ceilings,
//!   feature toggles read from environment),
//! - a logger target string the sweep prefixes its tracing spans
//!   with.
//!
//! **Per-backend handles (Vectorizer, Meili, Nexus, etc.) live on
//! the `impl Sweep for FooSweep` struct itself.** Constructors
//! inject them. The ctx is an environment, not a service locator.
//! This keeps the trait surface free of every backend SDK and
//! mirrors the existing `run_sweep(plan, ops)` shape in
//! `retention/mod.rs`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use cortex_storage::MetadataStore;
use tokio::sync::Mutex;

/// Thread-safe handle to the metadata store. Wraps
/// `Arc<Mutex<MetadataStore>>` because `MetadataStore`'s rusqlite
/// connection is `Send` but not `Sync`, and the scheduler hands the
/// same handle to every sweep across tokio tasks.
pub type MetadataHandle = Arc<Mutex<MetadataStore>>;

/// Shared knobs every sweep reads from the environment. The
/// `Default` impl carries the values the production daemon uses
/// today; tests override per-field.
#[derive(Debug, Clone)]
pub struct SweepConfig {
    /// Grace window (seconds) after which a `running`
    /// `retention_sweeps` row is treated as abandoned and the new
    /// invocation may proceed. Matches the existing
    /// `start_retention_sweep`-level constant so the trait does not
    /// introduce a second timeout. Default 3600 s (1 h).
    pub abandon_grace_secs: i64,
    /// Maximum records to process per backend round-trip. Per-sweep
    /// overrides live on the impl struct; this is the fallback.
    /// Default 256.
    pub default_batch_size: u32,
    /// `true` disables every mutation across the sweep set. Useful
    /// for `--dry-run` rollouts. Default `false`.
    pub dry_run: bool,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            abandon_grace_secs: 3600,
            default_batch_size: 256,
            dry_run: false,
        }
    }
}

/// Environment handed to every [`Sweep::run`](super::Sweep::run).
///
/// `Arc<MetadataStore>` because the scheduler hands the same ctx to
/// every sweep on every tick — cloning the handle is the cheap and
/// thread-safe choice.
///
/// `Debug` is hand-written because `MetadataStore` (the SQLite
/// connection wrapper) does not implement it.
#[derive(Clone)]
pub struct SweepCtx {
    /// SQLite-backed metadata store — owns `retention_sweeps`,
    /// `cron_jobs`, and the advisory locks the scheduler relies on.
    /// Wrapped in `Mutex` because the rusqlite `Connection` is
    /// `Send` but `!Sync`.
    pub metadata: MetadataHandle,
    /// Reference time. Production wires `Utc::now()`; tests pin to
    /// a fixed instant so age-based sweeps are deterministic.
    pub now: DateTime<Utc>,
    /// Shared configuration knobs (batch sizes, abandon grace,
    /// dry-run toggle).
    pub config: SweepConfig,
    /// Logger target string the sweep prefixes its tracing spans
    /// with — e.g., `"cortex.sweep.tier"`. Static lifetime so
    /// `tracing::info_span!(target: ...)` accepts it directly.
    pub logger_target: &'static str,
}

impl SweepCtx {
    /// Build a ctx with `Utc::now()` and default config — the shape
    /// the production scheduler instantiates per tick.
    pub fn new(metadata: MetadataHandle, logger_target: &'static str) -> Self {
        Self {
            metadata,
            now: Utc::now(),
            config: SweepConfig::default(),
            logger_target,
        }
    }

    /// Builder shim — pin the reference clock. Used by integration
    /// tests that time-travel.
    #[must_use]
    pub fn with_now(mut self, now: DateTime<Utc>) -> Self {
        self.now = now;
        self
    }

    /// Builder shim — override the shared config.
    #[must_use]
    pub fn with_config(mut self, config: SweepConfig) -> Self {
        self.config = config;
        self
    }
}

impl std::fmt::Debug for SweepCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SweepCtx")
            .field("metadata", &"<MetadataStore>")
            .field("now", &self.now)
            .field("config", &self.config)
            .field("logger_target", &self.logger_target)
            .finish()
    }
}
