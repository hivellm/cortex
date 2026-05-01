//! Storage helpers owned by `cortex-api`. Currently the only
//! occupant is [`api_keys`], the SQLite-backed table that powers
//! the phase3 §7 opt-in dashboard auth surface. Keep this module
//! tree minimal — most cortex-api state lives in the lane stack;
//! anything persisted to disk lands here.

#![allow(missing_docs)]

pub mod api_keys;
