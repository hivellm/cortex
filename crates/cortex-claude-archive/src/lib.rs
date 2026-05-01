//! `cortex-claude-archive` — phase11i §1.
//!
//! Walks Claude Code's `~/.claude/projects/<project>/*.jsonl`
//! conversation archive and emits canonical
//! [`cortex_core::events::Envelope`] records onto the existing
//! `cortex.events.bootstrap` Synap stream (or, optionally, into the
//! Parquet archive `cortex-api`'s [`archive_loader`] consumes at
//! boot). The pipeline downstream of this crate stays unchanged —
//! every classifier / embedder / fulltext / graph worker reads the
//! same envelopes the live `cortex-adapter-claude-code` already
//! ships.
//!
//! Module layout:
//!
//! - [`types`] — JSONL record shapes (`user`, `assistant`,
//!   `attachment`, …) with `serde::Deserialize` impls.
//! - [`reader`] — streaming line-by-line JSON reader tolerant to
//!   malformed records, incomplete final lines, and unknown
//!   subtype tags.
//! - [`mapper`] — pair user↔assistant records by `parentUuid` and
//!   project them into `Envelope { kind: Turn | ToolCall |
//!   AgentCall | Memory | Artifact, … }`.
//! - [`walker`] — directory traversal with exclude patterns
//!   (lands in §1.4).
//! - [`emitter`] — sinks: Synap publisher + Parquet archive
//!   writer (lands in §1.5).
//! - [`checkpoint`] — atomic `(project_dir, session_id,
//!   last_record_uuid, last_byte_offset)` write (lands in §1.6).
//!
//! Errors surface as [`ArchiveError`]; recoverable problems
//! (corrupt JSON line, unknown subtype) are logged and counted but
//! never panic.

#![forbid(unsafe_code)]

pub mod mapper;
pub mod reader;
pub mod types;

pub use mapper::{map_session, MapStats};
pub use reader::{read_records, ReadStats};
pub use types::{ArchiveError, JsonlRecord, RecordKind};
