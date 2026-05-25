//! Admin / operator endpoints — config audit, canary controls,
//! forget / list-events / silent-drop surfaces.
//!
//! Files in this bucket previously lived at
//! `crates/cortex-api/src/{admin_forget,admin_list_events,config_audit,canary,silent_drop}.rs`.
//! `admin_forget` and `admin_list_events` shed the redundant prefix;
//! the rest keep their names. External paths are preserved via
//! `pub use admin::<child>` re-exports in [`crate`] (see `lib.rs`).

pub mod canary;
pub mod config_audit;
pub mod forget;
pub mod list_events;
pub mod silent_drop;
