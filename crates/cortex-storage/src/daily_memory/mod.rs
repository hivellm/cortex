//! Compact, explicit daily memory in the existing metadata database.
//!
//! A local commit is immediately searchable without workers. Accepted lifecycle
//! is the existing core decision contract; classification never changes it here.
//! Durability covers process restart under the metadata store's WAL policy, not
//! an additional guarantee against power loss. Authorization and trusted origin
//! resolution belong to the API/adapter before these methods are called.

mod advice;
mod delivery;
mod types;

pub use advice::*;
pub use delivery::*;
pub use types::*;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use chrono::Utc;
use cortex_core::events::{DecisionPayload, Kind};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::{MetadataError, MetadataStore};

/// Errors distinguish invalid input, optimistic conflicts and backend failure.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// Invalid or oversized explicit input.
    #[error("invalid daily memory input: {0}")]
    InvalidInput(String),
    /// Requested record does not exist in the supplied scope.
    #[error("daily memory not found in requested scope")]
    NotFound,
    /// A create-only record ID already exists in this scope.
    #[error("daily memory ID already exists in requested scope")]
    AlreadyExists,
    /// The writer observed an obsolete scope revision.
    #[error("daily memory revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict {
        /// Revision supplied by the caller.
        expected: u64,
        /// Current revision within the authorized scope.
        actual: u64,
    },
    /// A caller reused a retry identity for a different request.
    #[error("daily memory retry key reused with different content")]
    IdempotencyConflict,
    /// Underlying SQLite failure, including missing required FTS5 support.
    #[error("daily memory sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Invalid persisted JSON or serialization failure.
    #[error("daily memory json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Apply additive daily-memory schema and require FTS5 at metadata open time.
pub(crate) fn apply_schema(conn: &Connection) -> Result<(), MetadataError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(include_str!("schema.sql"))?;
    tx.commit()?;
    Ok(())
}

impl MemoryScope {
    fn key(&self) -> Result<String, MemoryError> {
        match self {
            Self::Repository(repo) => {
                bounded("repo", repo, 256)?;
                Ok(format!("repo:{repo}"))
            }
            Self::Global => Ok("global".to_owned()),
        }
    }
}

impl MemoryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Superseded => "superseded",
            Self::Deprecated => "deprecated",
        }
    }
}

impl MetadataStore {
    /// Atomically capture explicit content, evidence, FTS, revision and retry receipt.
    ///
    /// Supersession requires an accepted predecessor and successor in the same
    /// scope and an expected scope revision. IDs are create-only within scope,
    /// so a correction cannot point back to an existing ancestor or itself.
    pub fn capture_daily_memory(
        &mut self,
        request: &MemoryCapture,
    ) -> Result<MemoryReceipt, MemoryError> {
        validate_capture(request)?;
        let scope = request.scope.key()?;
        let request_hash = hash(&serde_json::to_vec(request)?);
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(retry) = &request.retry {
            let previous: Option<(String, String)> = tx
                .query_row(
                    "SELECT request_hash,receipt FROM daily_memory_retries
                     WHERE scope=?1 AND caller=?2 AND retry_key=?3",
                    params![scope, retry.caller, retry.key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((previous_hash, receipt)) = previous {
                if previous_hash != request_hash {
                    return Err(MemoryError::IdempotencyConflict);
                }
                // Retry precedes optimistic checks: return the ORIGINAL receipt
                // even after another operation changes the record or revision.
                return Ok(serde_json::from_str(&receipt)?);
            }
        }
        let revision = revision(&tx, &scope)?;
        check_revision(request.expected_revision, revision)?;
        let id = request.id.clone().unwrap_or_else(cortex_core::event_id);
        if request.supersedes.as_deref() == Some(id.as_str()) {
            return Err(invalid("a memory cannot supersede itself"));
        }
        if read_record(&tx, &scope, &id)?.is_some() {
            return Err(MemoryError::AlreadyExists);
        }
        let now = Utc::now().to_rfc3339();
        if let Some(predecessor) = &request.supersedes {
            let mut prior = read_record(&tx, &scope, predecessor)?.ok_or(MemoryError::NotFound)?;
            if prior.status != MemoryStatus::Accepted {
                return Err(invalid("supersession requires an accepted predecessor"));
            }
            prior.status = MemoryStatus::Superseded;
            prior.version = next_version(prior.version)?;
            prior.content_hash = payload_hash(&prior)?;
            write_record(&tx, &prior)?;
            write_evidence(&tx, &prior, &request.origin, &now)?;
        }
        let event_id = cortex_core::event_id();
        let mut record = MemoryRecord {
            id,
            scope: request.scope.clone(),
            title: request.title.clone(),
            body: request.body.clone(),
            category: request.category,
            capture_kind: request.capture_kind,
            status: request.status,
            origin: request.origin.clone(),
            sources: request.sources.clone(),
            branch: request.branch.clone(),
            path: request.path.clone(),
            pinned: request.pinned,
            stale: false,
            version: 1,
            event_id: event_id.clone(),
            captured_at: now.clone(),
            content_hash: String::new(),
            supersedes: request.supersedes.clone(),
        };
        record.content_hash = payload_hash(&record)?;
        write_record(&tx, &record)?;
        write_evidence_with_id(&tx, &record, &request.origin, &now, &event_id)?;
        let receipt = receipt(&record, event_id, now, bump_revision(&tx, &scope)?);
        if let Some(retry) = &request.retry {
            tx.execute(
                "INSERT INTO daily_memory_retries(scope,caller,retry_key,request_hash,receipt)
                 VALUES(?1,?2,?3,?4,?5)",
                params![
                    scope,
                    retry.caller,
                    retry.key,
                    request_hash,
                    serde_json::to_string(&receipt)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(receipt)
    }

    /// Read an exact scope revision; zero means no memory has been committed.
    pub fn daily_memory_revision(&self, scope: &MemoryScope) -> Result<u64, MemoryError> {
        revision(self.conn(), &scope.key()?)
    }

    /// Look up current or historical-status content within an exact scope.
    pub fn daily_memory(
        &self,
        scope: &MemoryScope,
        id: &str,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        bounded("id", id, 256)?;
        read_record(self.conn(), &scope.key()?, id)
    }

    /// Read scoped canonical decision evidence in record-version order.
    pub fn daily_memory_evidence(
        &self,
        scope: &MemoryScope,
        id: &str,
    ) -> Result<Vec<MemoryEvidence>, MemoryError> {
        bounded("id", id, 256)?;
        let mut statement = self.conn().prepare(
            "SELECT data FROM daily_memory_evidence
             WHERE scope=?1 AND memory_id=?2 ORDER BY version",
        )?;
        let rows = statement.query_map(params![scope.key()?, id], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Change deterministic source validity, atomically invalidating scoped caches.
    ///
    /// Caller verifies the actual source and authorizes this operation; no model
    /// inference or filesystem rewriting occurs. An unchanged marker is a no-op.
    pub fn mark_daily_memory_stale(
        &mut self,
        scope: &MemoryScope,
        id: &str,
        stale: bool,
        expected_revision: u64,
        origin: &MemoryOrigin,
    ) -> Result<MemoryReceipt, MemoryError> {
        bounded("id", id, 256)?;
        validate_origin(origin)?;
        let key = scope.key()?;
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = revision(&tx, &key)?;
        check_revision(Some(expected_revision), current)?;
        let mut record = read_record(&tx, &key, id)?.ok_or(MemoryError::NotFound)?;
        if record.stale == stale {
            let data: String = tx.query_row(
                "SELECT data FROM daily_memory_evidence WHERE scope=?1 AND memory_id=?2 AND version=?3",
                params![key, id, sql_version(record.version)?],
                |row| row.get(0),
            )?;
            let evidence: MemoryEvidence = serde_json::from_str(&data)?;
            return Ok(receipt(
                &record,
                evidence.event_id,
                evidence.occurred_at,
                current,
            ));
        }
        record.stale = stale;
        record.version = next_version(record.version)?;
        record.content_hash = payload_hash(&record)?;
        write_record(&tx, &record)?;
        let now = Utc::now().to_rfc3339();
        let event_id = write_evidence(&tx, &record, origin, &now)?;
        let result = receipt(&record, event_id, now, bump_revision(&tx, &key)?);
        tx.commit()?;
        Ok(result)
    }

    /// Query eligible local memories and revisions within one read snapshot.
    ///
    /// Scope/lifecycle/branch/path eligibility is in SQL before LIMIT. Pinned
    /// records explicitly apply throughout scope; other rows require FTS match.
    pub fn query_daily_memories(&self, query: &MemoryQuery) -> Result<MemorySearch, MemoryError> {
        let scope = MemoryScope::Repository(query.repo.clone()).key()?;
        if query.query.len() > 4096 {
            return Err(invalid("query exceeds 4 KiB"));
        }
        optional_bounded("branch", query.branch.as_deref(), 512)?;
        if query.paths.len() > 32 {
            return Err(invalid("at most 32 applicability paths are allowed"));
        }
        for path in &query.paths {
            validate_path(path)?;
        }
        let terms = literal_fts_query(&query.query)?;
        let tx = self.conn().unchecked_transaction()?;
        let mut result = MemorySearch {
            revision: revision(&tx, &scope)?,
            global_revision: if query.include_global {
                Some(revision(&tx, "global")?)
            } else {
                None
            },
            records: Vec::new(),
        };
        let limit = query.limit.min(50);
        if limit == 0 {
            return Ok(result);
        }
        let paths = serde_json::to_string(&query.paths)?;
        // Bound membership before ranking; global is explicit, never a wildcard.
        const ELIGIBLE: &str = "((d.scope=?1) OR (?2 AND d.scope='global'))
             AND d.status='accepted' AND d.stale=0
             AND (d.branch IS NULL OR d.branch=?3)
             AND (d.path IS NULL OR EXISTS (
                 SELECT 1 FROM json_each(?4) p
                 WHERE p.value=d.path OR substr(p.value,1,length(d.path)+1)=d.path||'/'))";
        let pinned_sql = format!(
            "SELECT d.data FROM daily_memory_records d WHERE {ELIGIBLE}
             AND d.pinned=1 ORDER BY d.scope,d.id LIMIT ?5"
        );
        result.records = collect_records(
            &tx,
            &pinned_sql,
            params![
                scope,
                query.include_global,
                query.branch,
                paths,
                limit as i64
            ],
        )?;
        if !terms.is_empty() && result.records.len() < limit {
            let sql = format!(
                "SELECT d.data FROM daily_memory_fts
                 JOIN daily_memory_records d ON d.rowid=daily_memory_fts.rowid
                 WHERE {ELIGIBLE} AND d.pinned=0 AND daily_memory_fts MATCH ?6
                 ORDER BY bm25(daily_memory_fts),d.scope,d.id LIMIT ?5"
            );
            result.records.extend(collect_records(
                &tx,
                &sql,
                params![
                    scope,
                    query.include_global,
                    query.branch,
                    paths,
                    (limit - result.records.len()) as i64,
                    terms
                ],
            )?);
        }
        tx.commit()?;
        Ok(result)
    }
}

fn invalid(message: &str) -> MemoryError {
    MemoryError::InvalidInput(message.to_owned())
}

fn bounded(name: &str, value: &str, max: usize) -> Result<(), MemoryError> {
    if value.trim().is_empty() || value.trim() != value || value.len() > max || value.contains('\0')
    {
        return Err(invalid(&format!(
            "{name} must be nonempty, trimmed and at most {max} bytes"
        )));
    }
    Ok(())
}

fn optional_bounded(name: &str, value: Option<&str>, max: usize) -> Result<(), MemoryError> {
    value.map(|v| bounded(name, v, max)).transpose()?;
    Ok(())
}

fn validate_origin(origin: &MemoryOrigin) -> Result<(), MemoryError> {
    optional_bounded("agent", origin.agent.as_deref(), 256)?;
    optional_bounded("session_id", origin.session_id.as_deref(), 256)?;
    optional_bounded("turn_id", origin.turn_id.as_deref(), 256)
}

fn validate_path(path: &str) -> Result<(), MemoryError> {
    bounded("path", path, 2048)?;
    if path.contains(['\\', ':']) || path.split('/').any(|p| matches!(p, "" | "." | "..")) {
        return Err(invalid(
            "path must be repository-relative with '/' segment boundaries",
        ));
    }
    Ok(())
}

fn validate_capture(request: &MemoryCapture) -> Result<(), MemoryError> {
    request.scope.key()?;
    if request.capture_kind.is_some_and(|kind| {
        !matches!(
            kind,
            Kind::Memory | Kind::Knowledge | Kind::Learning | Kind::Decision
        )
    }) {
        return Err(invalid(
            "capture kind must identify explicit memory content",
        ));
    }
    optional_bounded("id", request.id.as_deref(), 256)?;
    bounded("title", &request.title, 512)?;
    if request.body.trim().is_empty() || request.body.len() > 8192 || request.body.contains('\0') {
        return Err(invalid("body must be nonempty and at most 8 KiB"));
    }
    validate_origin(&request.origin)?;
    optional_bounded("branch", request.branch.as_deref(), 512)?;
    if let Some(path) = &request.path {
        validate_path(path)?;
    }
    if request.sources.len() > 16 {
        return Err(invalid("at most 16 source references are allowed"));
    }
    for source in &request.sources {
        bounded("source event_id", &source.event_id, 256)?;
        optional_bounded("source reference", source.reference.as_deref(), 2048)?;
        optional_bounded("source content_hash", source.content_hash.as_deref(), 256)?;
        optional_bounded("source revision", source.revision.as_deref(), 256)?;
    }
    if let Some(retry) = &request.retry {
        bounded("retry caller", &retry.caller, 256)?;
        bounded("retry key", &retry.key, 256)?;
    }
    if request.status == MemoryStatus::Superseded {
        return Err(invalid(
            "superseded status requires an explicit successor operation",
        ));
    }
    if let Some(prior) = &request.supersedes {
        bounded("supersedes", prior, 256)?;
        if request.expected_revision.is_none() || request.status != MemoryStatus::Accepted {
            return Err(invalid(
                "supersession requires expected revision and accepted successor",
            ));
        }
    }
    Ok(())
}

fn literal_fts_query(query: &str) -> Result<String, MemoryError> {
    // Keep every Unicode mark category attached to its word. Splitting at a
    // combining accent changes NFD words before unicode61 can normalize them.
    static WORDS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"[\p{L}\p{N}][\p{L}\p{N}\p{M}]*").expect("static Unicode word pattern is valid")
    });
    let terms: BTreeSet<_> = WORDS
        .find_iter(query)
        .map(|word| word.as_str().to_lowercase())
        .collect();
    if terms.len() > 32 {
        return Err(invalid("query exceeds 32 distinct terms"));
    }
    Ok(terms
        .into_iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(" OR "))
}

fn revision(conn: &Connection, scope: &str) -> Result<u64, MemoryError> {
    let value: i64 = conn
        .query_row(
            "SELECT revision FROM daily_memory_revisions WHERE scope=?1",
            [scope],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    u64::try_from(value)
        .map_err(|_| MemoryError::Sqlite(rusqlite::Error::IntegralValueOutOfRange(0, value)))
}

fn sql_version(version: u64) -> Result<i64, MemoryError> {
    i64::try_from(version).map_err(|_| invalid("memory version exceeds SQLite integer range"))
}

fn next_version(version: u64) -> Result<u64, MemoryError> {
    let next = version
        .checked_add(1)
        .ok_or_else(|| invalid("memory version overflow"))?;
    sql_version(next)?;
    Ok(next)
}

fn check_revision(expected: Option<u64>, actual: u64) -> Result<(), MemoryError> {
    if let Some(expected) = expected {
        if expected != actual {
            return Err(MemoryError::RevisionConflict { expected, actual });
        }
    }
    Ok(())
}

fn bump_revision(conn: &Connection, scope: &str) -> Result<u64, MemoryError> {
    conn.execute(
        "INSERT INTO daily_memory_revisions(scope,revision) VALUES(?1,1)
         ON CONFLICT(scope) DO UPDATE SET revision=revision+1",
        [scope],
    )?;
    revision(conn, scope)
}

fn read_record(
    conn: &Connection,
    scope: &str,
    id: &str,
) -> Result<Option<MemoryRecord>, MemoryError> {
    let json: Option<String> = conn
        .query_row(
            "SELECT data FROM daily_memory_records WHERE scope=?1 AND id=?2",
            params![scope, id],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
}

fn write_record(conn: &Connection, record: &MemoryRecord) -> Result<(), MemoryError> {
    conn.execute(
        "INSERT INTO daily_memory_records(scope,id,title,body,status,stale,pinned,branch,path,data)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(scope,id) DO UPDATE SET title=excluded.title,body=excluded.body,
           status=excluded.status,stale=excluded.stale,pinned=excluded.pinned,
           branch=excluded.branch,path=excluded.path,data=excluded.data",
        params![
            record.scope.key()?,
            record.id,
            record.title,
            record.body,
            record.status.as_str(),
            record.stale,
            record.pinned,
            record.branch,
            record.path,
            serde_json::to_string(record)?
        ],
    )?;
    Ok(())
}

fn decision_payload(record: &MemoryRecord) -> DecisionPayload {
    let mut tags = vec![
        "daily-memory".to_owned(),
        match record.category {
            MemoryCategory::Decision => "decision".to_owned(),
            MemoryCategory::Lesson => "lesson".to_owned(),
        },
    ];
    if record.stale {
        tags.push("stale".to_owned());
    }
    DecisionPayload {
        decision_id: record.id.clone(),
        title: record.title.clone(),
        status: record.status.as_str().to_owned(),
        body: record.body.clone(),
        supersedes: record.supersedes.clone(),
        cas_ref: None,
        tags,
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn payload_hash(record: &MemoryRecord) -> Result<String, MemoryError> {
    Ok(hash(&serde_json::to_vec(&decision_payload(record))?))
}

fn write_evidence(
    conn: &Connection,
    record: &MemoryRecord,
    origin: &MemoryOrigin,
    now: &str,
) -> Result<String, MemoryError> {
    let event_id = cortex_core::event_id();
    write_evidence_with_id(conn, record, origin, now, &event_id)?;
    Ok(event_id)
}

fn write_evidence_with_id(
    conn: &Connection,
    record: &MemoryRecord,
    origin: &MemoryOrigin,
    now: &str,
    event_id: &str,
) -> Result<(), MemoryError> {
    let evidence = MemoryEvidence {
        capture_kind: record.capture_kind,
        event_id: event_id.to_owned(),
        kind: Kind::Decision,
        version: record.version,
        payload: decision_payload(record),
        origin: origin.clone(),
        sources: record.sources.clone(),
        occurred_at: now.to_owned(),
        stale: record.stale,
    };
    conn.execute(
        "INSERT INTO daily_memory_evidence(event_id,scope,memory_id,version,data) VALUES(?1,?2,?3,?4,?5)",
        params![event_id,record.scope.key()?,record.id,sql_version(record.version)?,serde_json::to_string(&evidence)?],
    )?;
    Ok(())
}

fn receipt(
    record: &MemoryRecord,
    event_id: String,
    captured_at: String,
    scope_revision: u64,
) -> MemoryReceipt {
    MemoryReceipt {
        id: record.id.clone(),
        event_id,
        content_hash: record.content_hash.clone(),
        captured_at,
        version: record.version,
        scope_revision,
    }
}

fn collect_records<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<MemoryRecord>, MemoryError> {
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map(params, |row| row.get::<_, String>(0))?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}
