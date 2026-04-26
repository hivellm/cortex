//! Cortex Claude Code adapter — hooks + local daemon (spec 10).
//!
//! The adapter captures every meaningful Claude Code session signal —
//! prompts, tool calls, tool results, session boundaries — and
//! republishes them as envelope-compliant events on
//! `cortex-core /v1/events/batch`. Synchronous hooks for pre-thinking
//! and law-check fail-open so the session never breaks even when
//! `cortex-api` is unreachable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod dispatcher;
pub mod events;
pub mod install;
pub mod ipc;
pub mod metrics;
pub mod publisher;
pub mod session;
pub mod sync_paths;
pub mod wal;

pub use config::{
    load_or_default as load_config, AdapterSection, AdapterToml, ConfigError, ExtraPattern,
    LawsSection, LoggingSection, PreThinkingSection, RedactionSection,
};
pub use dispatcher::{Dispatcher, HookResponse};
pub use events::{build_event, ClaudeEvent, HookFrame, HookKind};
pub use install::{
    install, install_with, uninstall, HookShim, InstallError, InstallOptions, InstallReport,
    Layout, HOOK_SHIMS,
};
pub use ipc::{dispatch_inline, serve, IpcBinding};
pub use metrics::Metrics;
pub use publisher::{spawn_flusher, HttpPublisher, MemoryPublisher, Publisher};
pub use session::{SessionManager, SessionState};
pub use sync_paths::{
    LawCheckRequest, LawCheckResult, PreThinkingRequest, PreThinkingResult, SyncClient, Violation,
};
pub use wal::{OverflowWal, WalError};
