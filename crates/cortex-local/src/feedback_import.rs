//! Source-preserving snapshots of unattributed legacy SQLite feedback.
use crate::{feedback::*, files, LocalError, LocalStore, Result};
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const TABLE: &str = "pre_thinking_feedback";
const MAX_DATABASE_BYTES: u64 = 128 * 1024 * 1024;
const FIELDS: [&str; 8] = [
    "query_id",
    "intent",
    "helpful",
    "files_cited",
    "rating",
    "free_text",
    "implicit_score",
    "recorded_at",
];

#[derive(Debug, Default, Serialize)]
pub struct FeedbackImportReport {
    pub source: String,
    pub dry_run: bool,
    pub total_rows: usize,
    pub rows_seen: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub validated: usize,
    pub rejected: usize,
    pub cumulative_rejected: usize,
    pub prior_missing: usize,
    pub prior_rejected: usize,
    pub prior_progress_verified: bool,
    pub remaining: usize,
    pub next_cursor: Option<String>,
    pub complete: bool,
    pub failures: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportCursor {
    source: String,
    snapshot_sha256: String,
    offset: usize,
    dry_run: bool,
    cumulative_rejected: usize,
    cumulative_store_rejected: usize,
}

fn invalid(message: impl Into<String>) -> LocalError {
    LocalError::Invalid(message.into())
}

fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    files::ensure_no_symlinks(path)?;
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_DATABASE_BYTES {
        return Err(invalid(
            "legacy feedback database exceeds 128 MiB or is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_DATABASE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DATABASE_BYTES {
        return Err(invalid("legacy feedback database grew beyond 128 MiB"));
    }
    Ok(bytes)
}

fn sidecars(path: &Path) -> Result<Vec<Option<Vec<u8>>>> {
    let mut state = Vec::new();
    for suffix in ["-wal", "-journal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = Path::new(&name);
        files::ensure_no_symlinks(sidecar)?;
        match fs::symlink_metadata(sidecar) {
            Ok(metadata) => {
                if !metadata.is_file() || (suffix != "-shm" && metadata.len() != 0) {
                    return Err(invalid("legacy feedback import requires checkpointed database without pending WAL/journal"));
                }
                state.push(Some(bounded_read(sidecar)?));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => state.push(None),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(state)
}

fn normalized_path(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| invalid("legacy feedback source path must be UTF-8"))?;
    #[cfg(windows)]
    {
        Ok(text
            .strip_prefix("\\\\?\\")
            .unwrap_or(text)
            .replace('\\', "/")
            .to_lowercase())
    }
    #[cfg(not(windows))]
    {
        Ok(text.to_owned())
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("String formatting");
    }
    result
}

fn raw_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => json!({"storage_class":"null","value":null}),
        ValueRef::Integer(value) => json!({"storage_class":"integer","value":value.to_string()}),
        ValueRef::Real(value) => {
            json!({"storage_class":"real","value":format!("{:016x}",value.to_bits())})
        }
        ValueRef::Text(value) => json!({"storage_class":"text","value":hex(value)}),
        ValueRef::Blob(value) => json!({"storage_class":"blob","value":hex(value)}),
    }
}

fn row_label(position: usize, key: ValueRef<'_>) -> String {
    let mut hash = Sha256::new();
    match key {
        ValueRef::Null => hash.update(b"null"),
        ValueRef::Integer(value) => {
            hash.update(b"integer\0");
            hash.update(value.to_le_bytes());
        }
        ValueRef::Real(value) => {
            hash.update(b"real\0");
            hash.update(value.to_bits().to_le_bytes());
        }
        ValueRef::Text(value) => {
            hash.update(b"text\0");
            hash.update(value);
        }
        ValueRef::Blob(value) => {
            hash.update(b"blob\0");
            hash.update(value);
        }
    }
    format!("row {position} key_sha256={:x}", hash.finalize())
}

fn prepare_row(row: &rusqlite::Row<'_>, source_name: &str) -> Result<(FeedbackRequest, Value)> {
    let mut raw = serde_json::Map::new();
    let mut size = 0usize;
    for (index, field) in FIELDS.iter().enumerate() {
        let value = row.get_ref(index)?;
        size += match value {
            ValueRef::Text(v) | ValueRef::Blob(v) => v.len(),
            _ => 16,
        };
        if size > 64 * 1024 {
            break;
        }
        raw.insert((*field).into(), raw_value(value));
    }
    if size > 64 * 1024 || raw.len() != 8 {
        return Err(invalid("exceeds feedback size bound"));
    }
    if matches!(row.get_ref(0)?, ValueRef::Null) {
        return Err(invalid("has NULL query_id"));
    }
    let raw = Value::Object(raw);
    let legacy = json!({"source_path":source_name,"table":TABLE,"raw_row_sha256":files::hash(&files::canonical(&raw)?),"raw_row":raw});
    let id = format!(
        "legacy-feedback-{}",
        files::hash(&files::canonical(&json!([
            source_name,
            TABLE,
            legacy["raw_row"]
        ]))?)
    );
    let request = FeedbackRequest {
        id,
        repository: "legacy-feedback-unattributed".into(),
        source: FeedbackSource {
            kind: "legacy_sqlite".into(),
            reference: source_name.to_owned(),
            actor: "unknown".into(),
        },
        evidence: FeedbackEvidence::LegacyUnknown,
        judgment: FeedbackJudgment::Unknown,
        query_id: None,
        delivery_id: None,
        reference: None,
        comment: None,
    };
    validate_legacy_request(&request, &legacy)?;
    Ok((request, legacy))
}

/// Reads a bounded private snapshot; SQLite never opens the original database.
/// Two byte comparisons and sidecar checks reject changes during snapshot parsing.
pub fn import_feedback(
    store: &LocalStore,
    source: &Path,
    dry_run: bool,
    max_records: usize,
) -> Result<FeedbackImportReport> {
    import_feedback_page(store, source, dry_run, max_records, None)
}

/// Continue only against the exact source snapshot named by the opaque cursor.
pub fn import_feedback_page(
    store: &LocalStore,
    source: &Path,
    dry_run: bool,
    max_records: usize,
    cursor: Option<&str>,
) -> Result<FeedbackImportReport> {
    if !(1..=10000).contains(&max_records) {
        return Err(invalid("max_records must be 1..10000"));
    }
    files::ensure_no_symlinks(source)?;
    let source = fs::canonicalize(source)?;
    let source_name = normalized_path(&source)?;
    let before_sidecars = sidecars(&source)?;
    let before_metadata = fs::metadata(&source)?;
    let bytes = bounded_read(&source)?;
    let snapshot_sha256 = files::hash(&bytes);
    let (offset, prior_rejected, prior_store_rejected) = match cursor {
        Some(cursor) => {
            if cursor.len() > 8192 {
                return Err(invalid("feedback import cursor exceeds 8192 bytes"));
            }
            let cursor: ImportCursor = serde_json::from_str(cursor)?;
            if cursor.source != source_name || cursor.snapshot_sha256 != snapshot_sha256 {
                return Err(invalid("feedback import cursor source or snapshot changed"));
            }
            if cursor.dry_run != dry_run {
                return Err(invalid(
                    "feedback import cursor mode differs from requested dry_run",
                ));
            }
            if cursor.cumulative_rejected > cursor.offset {
                return Err(invalid(
                    "feedback import cursor rejection count exceeds processed rows",
                ));
            }
            if cursor.cumulative_store_rejected > cursor.cumulative_rejected
                || (dry_run && cursor.cumulative_store_rejected != 0)
            {
                return Err(invalid(
                    "feedback import cursor store rejection count is invalid",
                ));
            }
            (
                cursor.offset,
                cursor.cumulative_rejected,
                cursor.cumulative_store_rejected,
            )
        }
        None => (0, 0, 0),
    };
    let temporary = tempfile::tempdir()?;
    let copy_path = temporary.path().join("snapshot.sqlite");
    let mut copy = fs::File::create(&copy_path)?;
    copy.write_all(&bytes)?;
    copy.flush()?;
    drop(copy);
    let connection = Connection::open_with_flags(&copy_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.pragma_update(None, "query_only", true)?;
    let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(invalid("legacy feedback snapshot integrity check failed"));
    }
    let mut schema = connection.prepare("PRAGMA table_xinfo(pre_thinking_feedback)")?;
    let columns = schema
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if columns.len() != FIELDS.len()
        || columns.iter().any(|(_, _, hidden)| *hidden != 0)
        || FIELDS
            .iter()
            .any(|field| !columns.iter().any(|(name, _, _)| name == field))
        || columns
            .iter()
            .filter(|(_, pk, _)| *pk != 0)
            .map(|(name, _, _)| name.as_str())
            .collect::<Vec<_>>()
            != vec!["query_id"]
    {
        return Err(invalid(
            "unsupported legacy feedback schema or query_id primary key",
        ));
    }
    let without_rowid: bool = connection.query_row(
        "SELECT wr FROM pragma_table_list WHERE schema='main' AND name='pre_thinking_feedback' AND type='table'",
        [], |row| row.get(0),
    )?;
    let ordering = if without_rowid {
        "query_id"
    } else {
        "query_id, rowid"
    };
    let total: i64 =
        connection.query_row("SELECT count(*) FROM pre_thinking_feedback", [], |row| {
            row.get(0)
        })?;
    let mut report = FeedbackImportReport {
        source: source_name.clone(),
        dry_run,
        total_rows: usize::try_from(total).map_err(|_| invalid("invalid row count"))?,
        cumulative_rejected: prior_rejected,
        ..Default::default()
    };
    // Borrow raw values and reject oversized rows before encoding or copying payloads.
    if offset > report.total_rows {
        return Err(invalid(
            "feedback import cursor offset exceeds snapshot row count",
        ));
    }
    let mut statement = connection.prepare(&format!("SELECT query_id,intent,helpful,files_cited,rating,free_text,implicit_score,recorded_at FROM pre_thinking_feedback ORDER BY {ordering} LIMIT ?1 OFFSET ?2"))?;
    let mut rows = statement.query([
        max_records as i64,
        i64::try_from(offset).map_err(|_| invalid("feedback import cursor offset overflow"))?,
    ])?;
    let mut prepared = Vec::new();
    while let Some(row) = rows.next()? {
        report.rows_seen += 1;
        let row_label = row_label(offset + report.rows_seen, row.get_ref(0)?);
        match prepare_row(row, &source_name) {
            Ok((request, legacy)) => prepared.push((row_label, request, legacy)),
            Err(error) => {
                report.rejected += 1;
                report.failures.push(format!("{row_label}: {error}"));
            }
        }
    }
    let after_metadata = fs::metadata(&source)?;
    if before_metadata.len() != after_metadata.len()
        || before_metadata.modified()? != after_metadata.modified()?
        || bounded_read(&source)? != bytes
        || sidecars(&source)? != before_sidecars
    {
        return Err(invalid(
            "legacy feedback source changed during snapshot inspection",
        ));
    }
    let next_offset = offset + report.rows_seen;
    report.remaining = report.total_rows.saturating_sub(next_offset);
    if report.remaining == 0 {
        // A caller-controlled cursor cannot prove earlier pages reached this store.
        let destination = if dry_run {
            None
        } else {
            Some(store.readonly_index()?)
        };
        let mut prefix = connection.prepare(&format!("SELECT query_id,intent,helpful,files_cited,rating,free_text,implicit_score,recorded_at FROM pre_thinking_feedback ORDER BY {ordering} LIMIT ?1"))?;
        let mut prefix_rows = prefix.query([offset as i64])?;
        let mut position = 0;
        while let Some(row) = prefix_rows.next()? {
            position += 1;
            let label = row_label(position, row.get_ref(0)?);
            match prepare_row(row, &source_name) {
                Err(error) => {
                    report.prior_rejected += 1;
                    report
                        .failures
                        .push(format!("{label}: prior row rejected: {error}"));
                }
                Ok((request, legacy)) => {
                    if let Some(destination) = &destination {
                        match crate::feedback::get(
                            destination.connection(),
                            &request.repository,
                            &request.id,
                        ) {
                            Ok(Some(record))
                                if record.request == request
                                    && record.legacy.as_ref() == Some(&legacy) => {}
                            Ok(_) => {
                                report.prior_missing += 1;
                                report.failures.push(format!(
                                    "{label}: prior row missing or changed in destination"
                                ));
                            }
                            Err(error) => {
                                report.prior_missing += 1;
                                report.failures.push(format!(
                                    "{label}: prior destination read failed: {error}"
                                ));
                            }
                        }
                    }
                }
            }
        }
        report.prior_progress_verified = report.prior_missing == 0
            && report.prior_rejected == prior_rejected - prior_store_rejected;
        if report.prior_rejected != prior_rejected - prior_store_rejected {
            report
                .failures
                .push("cursor rejection count differs from revalidated prefix".into());
        }
    }
    let mut store_rejected = 0usize;
    for (row_label, request, legacy) in prepared {
        if dry_run {
            report.validated += 1;
            continue;
        }
        match store.import_feedback(request, legacy) {
            Ok(outcome) if outcome.created => report.imported += 1,
            Ok(_) => report.duplicates += 1,
            Err(error) => {
                report.rejected += 1;
                store_rejected += 1;
                report.failures.push(format!("{row_label}: {error}"));
            }
        }
    }
    report.cumulative_rejected = prior_rejected
        .max(report.prior_rejected + prior_store_rejected)
        .checked_add(report.rejected)
        .ok_or_else(|| invalid("feedback rejection count overflow"))?;
    report.remaining = report.total_rows.saturating_sub(next_offset);
    if report.remaining > 0 {
        report.next_cursor = Some(serde_json::to_string(&ImportCursor {
            source: source_name,
            snapshot_sha256,
            offset: next_offset,
            dry_run,
            cumulative_rejected: report.cumulative_rejected,
            cumulative_store_rejected: prior_store_rejected + store_rejected,
        })?);
    }
    report.complete =
        report.remaining == 0 && report.cumulative_rejected == 0 && report.prior_progress_verified;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_storage_tag_requires_value_key_even_with_same_field_count() {
        let db = Connection::open_in_memory().unwrap();
        let mut statement = db
            .prepare("SELECT 'q1',NULL,1,'[]',5,'old',0.5,'time'")
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        let (request, mut legacy) =
            prepare_row(rows.next().unwrap().unwrap(), "/legacy.sqlite").unwrap();
        let tag = legacy["raw_row"]["intent"].as_object_mut().unwrap();
        tag.remove("value");
        tag.insert("unrecognized".into(), Value::Null);
        let error = validate_legacy_request(&request, &legacy).unwrap_err();
        assert!(error.to_string().contains("legacy storage fields mismatch"));
    }
}
