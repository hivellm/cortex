//! Post-retrieval phantom-link verifier (phase17 §3).
//!
//! Checks whether a cited `(path, symbol)` pair actually exists in the
//! repository before surfacing it to the model. The concrete resolver
//! lives in [`symbols`].

pub mod symbols;

pub use symbols::{verify_symbol, SymbolVerdict};
