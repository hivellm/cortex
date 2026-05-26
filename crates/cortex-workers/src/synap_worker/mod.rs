//! Phase14h — shared Synap-consumer worker scaffolding.
//!
//! Closes glm5.1 F-003: the four event-pipeline workers
//! (embedder / fulltext / graph / classifier) all carried a
//! near-identical `run_once` / `run_forever` / `run_pool` triplet
//! plus per-worker copies of the supervisor / back-off / idle-sleep
//! logic. A bug fix in one rarely landed in the other three.
//!
//! This module exposes:
//!
//! - [`SynapWorker`] — minimal trait every worker implements
//!   (`worker_name`, `pool_size`, `run_once`).
//! - [`run_forever`] / [`run_pool`] — generic drivers that own
//!   the loop shape (back-off, supervisor, idle sleep, graceful
//!   shutdown). Per-worker variations are exposed as defaulted
//!   trait hooks so the trait stays small.
//! - [`metrics::WorkerMetrics`] — shared lag gauge +
//!   dead-letter counter family keyed by `{worker, reason}`.
//! - [`dead_letter`] — typed dead-letter sink with a fixed
//!   reason taxonomy.
//! - [`checkpoint::CursorCheckpoint`] — persists the last
//!   ack'd offset to `producer_checkpoints` so a kill-resume
//!   cycle does not rewind to offset 0.

pub mod checkpoint;
pub mod dead_letter;
pub mod metrics;
pub mod runtime;
#[allow(clippy::module_inception)]
mod trait_def;

pub use runtime::{run_forever, run_pool, RunError};
pub use trait_def::{BackpressureGate, SynapWorker};
