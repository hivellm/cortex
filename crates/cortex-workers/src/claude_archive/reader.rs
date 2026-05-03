//! Streaming JSONL reader — phase11i §1.2.
//!
//! Walks a session file line by line. Each successful parse yields
//! a [`JsonlRecord`]; malformed lines are logged and counted.
//! Tolerant to:
//!
//! - Incomplete final line (a session that crashed mid-write).
//! - Out-of-order `parentUuid` chains (the mapper resolves
//!   ordering itself — the reader stays a flat stream).
//! - Unknown `type` tags (folded into `RecordKind::Unknown(..)`;
//!   never panics).
//! - UTF-8 errors mid-file (skip + warn + continue).
//!
//! The reader does NOT load the whole file into memory; sessions
//! up to the largest observed (1.4 GB) stream through a 64 KiB
//! `BufReader`.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::types::{ArchiveError, JsonlRecord, RecordKind};

/// Aggregate counters returned alongside each successful read.
/// Surfaced to the caller (CLI / watcher) for telemetry.
#[derive(Debug, Default, Clone)]
pub struct ReadStats {
    /// Total lines consumed (including blanks + malformed).
    pub lines_read: usize,
    /// Lines that parsed cleanly into a [`JsonlRecord`].
    pub records_parsed: usize,
    /// Lines that failed JSON parsing.
    pub malformed_lines: usize,
    /// Records carrying a `type` outside the seven canonical
    /// shapes — counted but still passed through (`RecordKind::Unknown`).
    pub unknown_kinds: usize,
    /// Records with no `type` field at all (treated as malformed).
    pub typeless_records: usize,
}

/// Read a session JSONL file end-to-end. Returns the parsed
/// records (in file order) plus a [`ReadStats`] summary. Recoverable
/// errors are absorbed; unrecoverable I/O failures bubble out as
/// [`ArchiveError::Io`].
pub fn read_records(path: impl AsRef<Path>) -> Result<(Vec<JsonlRecord>, ReadStats), ArchiveError> {
    let path_ref = path.as_ref();
    let path_str = path_ref.to_string_lossy().to_string();
    let file = File::open(path_ref).map_err(|source| ArchiveError::Io {
        path: path_str.clone(),
        source,
    })?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    parse_lines(reader, &path_str)
}

/// Variant that takes any `BufRead` — used by tests + the watcher
/// (which pipes a `notify` re-read into the same parser).
pub fn parse_lines<R: BufRead>(
    reader: R,
    path_for_errors: &str,
) -> Result<(Vec<JsonlRecord>, ReadStats), ArchiveError> {
    let mut records = Vec::new();
    let mut stats = ReadStats::default();
    for (idx, line_result) in reader.lines().enumerate() {
        let line_num = idx + 1;
        stats.lines_read += 1;
        let line = match line_result {
            Ok(s) => s,
            Err(_) => {
                // Likely a UTF-8 decode error (rare in this corpus
                // but possible on copy-pasted binary tool output).
                // Count + skip.
                stats.malformed_lines += 1;
                continue;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<JsonlRecord>(trimmed) {
            Ok(record) => {
                let Some(kind) = record.kind.as_deref() else {
                    stats.typeless_records += 1;
                    tracing::warn!(
                        path = path_for_errors,
                        line = line_num,
                        "JSONL record has missing `type` field; skipping",
                    );
                    continue;
                };
                if kind.is_empty() {
                    stats.typeless_records += 1;
                    continue;
                }
                if matches!(RecordKind::from_tag(kind), RecordKind::Unknown(_)) {
                    stats.unknown_kinds += 1;
                    tracing::debug!(
                        path = path_for_errors,
                        line = line_num,
                        kind = kind,
                        "JSONL record has unknown `type`; passing through as Unknown",
                    );
                }
                stats.records_parsed += 1;
                records.push(record);
            }
            Err(source) => {
                stats.malformed_lines += 1;
                tracing::warn!(
                    path = path_for_errors,
                    line = line_num,
                    err = %source,
                    "JSONL line failed to parse; skipping",
                );
            }
        }
    }
    Ok((records, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(s: &str) -> (Vec<JsonlRecord>, ReadStats) {
        parse_lines(Cursor::new(s), "<test>").expect("test parse should never fail I/O")
    }

    #[test]
    fn empty_input_returns_empty_stats() {
        let (records, stats) = parse("");
        assert!(records.is_empty());
        assert_eq!(stats.records_parsed, 0);
        assert_eq!(stats.malformed_lines, 0);
    }

    #[test]
    fn single_user_record_parses() {
        let line = r#"{"type":"user","sessionId":"s1","uuid":"u1","timestamp":"2026-04-20T17:47:59.616Z","cwd":"E:\\HiveLLM\\Rulebook","gitBranch":"main","version":"2.1.112","entrypoint":"claude-vscode","userType":"external","message":{"role":"user","content":"hi"}}"#;
        let (records, stats) = parse(line);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind.as_deref(), Some("user"));
        assert_eq!(records[0].session_id.as_deref(), Some("s1"));
        assert_eq!(stats.records_parsed, 1);
        assert_eq!(stats.malformed_lines, 0);
    }

    #[test]
    fn malformed_line_is_counted_and_skipped() {
        let input = "not valid json\n{\"type\":\"user\",\"sessionId\":\"s2\"}\n";
        let (records, stats) = parse(input);
        assert_eq!(records.len(), 1);
        assert_eq!(stats.records_parsed, 1);
        assert_eq!(stats.malformed_lines, 1);
        assert_eq!(stats.lines_read, 2);
    }

    #[test]
    fn blank_lines_are_silently_dropped() {
        let input = "\n\n{\"type\":\"user\",\"sessionId\":\"s3\"}\n\n";
        let (records, stats) = parse(input);
        assert_eq!(records.len(), 1);
        assert_eq!(stats.malformed_lines, 0);
    }

    #[test]
    fn unknown_kind_passes_through_as_record_kind_unknown() {
        let line = r#"{"type":"future-shape","sessionId":"s4"}"#;
        let (records, stats) = parse(line);
        assert_eq!(records.len(), 1);
        assert_eq!(stats.unknown_kinds, 1);
        assert_eq!(stats.records_parsed, 1);
        assert!(matches!(
            RecordKind::from_tag(records[0].kind.as_deref().unwrap_or("")),
            RecordKind::Unknown(_)
        ));
    }

    #[test]
    fn record_without_type_field_is_dropped_as_typeless() {
        let line = r#"{"sessionId":"s5"}"#;
        let (records, stats) = parse(line);
        assert!(records.is_empty());
        assert_eq!(stats.typeless_records, 1);
    }

    #[test]
    fn incomplete_final_line_does_not_crash_the_reader() {
        // Trailing partial JSON (no newline) — the lines() iterator
        // yields it; serde_json rejects; we count + skip.
        let input = "{\"type\":\"user\",\"sessionId\":\"s6\"}\n{\"type\":\"a";
        let (records, stats) = parse(input);
        assert_eq!(records.len(), 1);
        assert_eq!(stats.malformed_lines, 1);
    }

    #[test]
    fn assistant_record_carries_message_block_intact() {
        let line = r#"{"type":"assistant","sessionId":"s7","uuid":"u7","parentUuid":"u6","timestamp":"2026-04-20T17:48:02.667Z","cwd":"e:\\HiveLLM\\Cortex","gitBranch":"main","version":"2.1.120","message":{"model":"claude-opus-4-7","id":"msg_x","type":"message","role":"assistant","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":6,"output_tokens":16}},"requestId":"req_x"}"#;
        let (records, _) = parse(line);
        assert_eq!(records.len(), 1);
        let msg = records[0].message.as_ref().expect("message block");
        assert_eq!(msg["model"], serde_json::json!("claude-opus-4-7"));
        assert_eq!(msg["role"], serde_json::json!("assistant"));
    }

    #[test]
    fn attachment_record_preserves_subtype() {
        let line = r#"{"type":"attachment","sessionId":"s8","uuid":"u8","timestamp":"2026-04-20T17:47:59.162Z","attachment":{"type":"hook_success","hookEvent":"SessionStart","exitCode":0}}"#;
        let (records, _) = parse(line);
        assert_eq!(records.len(), 1);
        let att = records[0].attachment.as_ref().expect("attachment block");
        assert_eq!(att["type"], serde_json::json!("hook_success"));
    }
}
