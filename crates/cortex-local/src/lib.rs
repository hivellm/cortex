//! Standalone local Cortex engine shared by CLI, MCP, hooks and web transports.
#![forbid(unsafe_code)]

pub mod audit;
pub mod classification;
pub mod files;
pub mod graph;
pub mod hooks;
pub mod import;
pub mod index;
pub mod install;
pub mod mcp;
pub mod model;
mod readonly;
pub mod recall;
pub mod runtime;
pub mod store;
pub mod upgrade;
pub mod web;

pub use audit::*;
pub use model::*;
pub use store::LocalStore;
