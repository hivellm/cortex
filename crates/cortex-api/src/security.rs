//! Security subsystem — request authentication / authorisation,
//! ACL evaluation, audit log, rate limiting, redaction.
//!
//! Every file in this bucket previously lived at
//! `crates/cortex-api/src/<name>.rs`. The bucketing is a 1:1 rename
//! to give the module tree a thematic home; external paths are
//! preserved via `pub use security::<child>` re-exports in
//! [`crate`] (see `lib.rs`).

pub mod acl;
pub mod audit;
pub mod audit_store;
pub mod auth;
pub mod principal;
pub mod principal_resolver;
pub mod principal_store;
pub mod rate_limit;
pub mod redaction;
