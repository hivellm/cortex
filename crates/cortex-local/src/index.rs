//! SQLite is a disposable projection of canonical records.
use crate::{files, model::*};
use rusqlite::{params, Connection, OptionalExtension};
use std::{path::Path, time::Duration};

/// Open only an already initialized projection; hooks must never run schema DDL.
pub(crate) fn connect_existing(root: &Path, timeout: Duration) -> Result<Connection> {
    let path = root.join("index.sqlite3");
    files::ensure_no_symlinks(&path)?;
    if !path.is_file() {
        return Err(LocalError::Invalid(
            "hook index is missing; run reindex --repair".into(),
        ));
    }
    for suffix in [
        "index.sqlite3-wal",
        "index.sqlite3-shm",
        "index.sqlite3-journal",
    ] {
        let sidecar = root.join(suffix);
        files::ensure_no_symlinks(&sidecar)?;
        if sidecar.exists()
            && (!sidecar.is_file()
                || (suffix != "index.sqlite3-shm" && sidecar.metadata()?.len() > 0))
        {
            return Err(LocalError::Invalid(
                "hook index requires explicit journal recovery".into(),
            ));
        }
    }
    let connection =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    connection.busy_timeout(timeout)?;
    validate_schema(&connection)?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(connection)
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<()> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != 3 {
        return Err(LocalError::Invalid(
            "hook index schema unavailable; run reindex --repair".into(),
        ));
    }
    for query in [
        "SELECT repository,id,revision,hash,record FROM revisions LIMIT 0",
        "SELECT rowid,repository,id,revision,active,kind,observed_at FROM current_records LIMIT 0",
        "SELECT rowid,title,body FROM record_fts LIMIT 0",
        "SELECT repository,source_id,target_id,relation FROM edges LIMIT 0",
        "SELECT repository,id,recorded_at,record FROM audit_records LIMIT 0",
        "SELECT repository,delivery_id FROM audit_archived LIMIT 0",
        "SELECT repository,id,recorded_at,record FROM feedback_records LIMIT 0",
    ] {
        connection.prepare(query)?;
    }
    Ok(())
}

pub(crate) fn recent(
    connection: &Connection,
    repository: &str,
    limit: usize,
    include_inactive: bool,
    memories_only: bool,
) -> Result<Vec<Record>> {
    files::validate_scope(repository)?;
    if !(1..=MAX_SEARCH_RESULTS).contains(&limit) {
        return Err(LocalError::Invalid("recent limit must be 1..100".into()));
    }
    let mut statement = connection.prepare("SELECT r.record FROM current_records c JOIN revisions r USING(repository,id,revision) WHERE c.repository=?1 AND (?2 OR c.active=1) AND (NOT ?4 OR c.kind!='\"event\"') AND (?2 OR ((json_type(r.record,'$.metadata.applicability.stale') IS NULL OR json_type(r.record,'$.metadata.applicability.stale') IN ('null','false')) AND json_extract(r.record,'$.metadata.applicability.branch') IS NULL AND json_extract(r.record,'$.metadata.applicability.path') IS NULL)) ORDER BY c.observed_at DESC,c.id ASC LIMIT ?3")?;
    let rows = statement.query_map(
        params![repository, include_inactive, limit as i64, memories_only],
        |row| row.get::<_, String>(0),
    )?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

pub(crate) fn connect(
    root: &Path,
    busy_timeout: Duration,
    before_initialize: impl FnOnce() -> Result<()>,
) -> Result<(Connection, bool)> {
    let path = root.join("index.sqlite3");
    files::ensure_no_symlinks(&path)?;
    for suffix in ["index.sqlite3-wal", "index.sqlite3-shm"] {
        files::ensure_no_symlinks(&root.join(suffix))?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(busy_timeout.min(Duration::from_secs(30)))?;
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > 3 {
        return Err(LocalError::Invalid(format!(
            "index schema {version} is newer than supported schema 3"
        )));
    }
    let tables: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('revisions','current_records','record_fts','edges','audit_records','audit_archived','feedback_records')",
        [], |row| row.get(0))?;
    let uninitialized = version < 3 || tables != 7;
    if uninitialized {
        before_initialize()?;
    }
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
    if uninitialized {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS revisions (
          repository TEXT NOT NULL, id TEXT NOT NULL, revision INTEGER NOT NULL,
          hash TEXT NOT NULL, record TEXT NOT NULL,
          PRIMARY KEY(repository,id,revision));
        CREATE TABLE IF NOT EXISTS current_records (
          rowid INTEGER PRIMARY KEY, repository TEXT NOT NULL, id TEXT NOT NULL,
          revision INTEGER NOT NULL, active INTEGER NOT NULL, kind TEXT NOT NULL,
          observed_at INTEGER NOT NULL,
          UNIQUE(repository,id));
        CREATE INDEX IF NOT EXISTS current_scope ON current_records(repository,active);
        CREATE VIRTUAL TABLE IF NOT EXISTS record_fts USING fts5(title,body,tokenize='unicode61');
        CREATE TABLE IF NOT EXISTS edges (
          repository TEXT NOT NULL, source_id TEXT NOT NULL, target_id TEXT NOT NULL,
          relation TEXT NOT NULL,
          PRIMARY KEY(repository,source_id,target_id,relation));
        CREATE INDEX IF NOT EXISTS incoming_edges ON edges(repository,target_id);
        CREATE TABLE IF NOT EXISTS audit_records (
          repository TEXT NOT NULL, id TEXT NOT NULL, recorded_at TEXT NOT NULL,
          record TEXT NOT NULL, PRIMARY KEY(repository,id));
        CREATE INDEX IF NOT EXISTS audit_scope_time ON audit_records(repository,recorded_at,id);
        CREATE TABLE IF NOT EXISTS audit_archived (
          repository TEXT NOT NULL, delivery_id TEXT NOT NULL,
          PRIMARY KEY(repository,delivery_id));
        CREATE TABLE IF NOT EXISTS feedback_records (
          repository TEXT NOT NULL, id TEXT NOT NULL, recorded_at TEXT NOT NULL,
          record TEXT NOT NULL, PRIMARY KEY(repository,id));
        CREATE INDEX IF NOT EXISTS feedback_scope_time ON feedback_records(repository,recorded_at,id);",
        )?;
    }
    Ok((connection, uninitialized))
}

pub(crate) fn get(
    connection: &Connection,
    repository: &str,
    id: &str,
    revision: Option<u64>,
) -> Result<Option<Record>> {
    let json: Option<String> = match revision {
        Some(revision) => connection.query_row(
            "SELECT record FROM revisions WHERE repository=?1 AND id=?2 AND revision=?3",
            params![repository, id, revision as i64], |row| row.get(0)).optional()?,
        None => connection.query_row(
            "SELECT r.record FROM revisions r JOIN current_records c USING(repository,id,revision) WHERE c.repository=?1 AND c.id=?2",
            params![repository, id], |row| row.get(0)).optional()?,
    };
    json.map(|value| serde_json::from_str(&value).map_err(LocalError::from))
        .transpose()
}

pub(crate) fn project(connection: &Connection, record: &Record) -> Result<()> {
    if let Some(existing) = get(
        connection,
        &record.repository,
        &record.id,
        Some(record.revision),
    )? {
        if existing != *record {
            return Err(LocalError::Conflict(format!(
                "conflicting canonical revision {}:{}",
                record.id, record.revision
            )));
        }
    } else {
        connection.execute(
            "INSERT INTO revisions(repository,id,revision,hash,record) VALUES(?1,?2,?3,?4,?5)",
            params![
                record.repository,
                record.id,
                record.revision as i64,
                record.content_sha256,
                serde_json::to_string(record)?
            ],
        )?;
    }
    let current: Option<(i64, i64)> = connection
        .query_row(
            "SELECT rowid,revision FROM current_records WHERE repository=?1 AND id=?2",
            params![record.repository, record.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if current.is_some_and(|(_, version)| version >= record.revision as i64) {
        return Ok(());
    }
    let kind = serde_json::to_string(&record.kind)?;
    let observed_at = record
        .source
        .captured_at
        .as_deref()
        .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
        .or_else(|| chrono::DateTime::parse_from_rfc3339(&record.created_at).ok())
        .ok_or_else(|| LocalError::Invalid("record has no valid capture time".into()))?
        .timestamp_millis();
    let rowid = if let Some((rowid, _)) = current {
        connection.execute("DELETE FROM record_fts WHERE rowid=?1", [rowid])?;
        connection.execute("UPDATE current_records SET revision=?1,active=?2,kind=?3,observed_at=?4 WHERE rowid=?5",
            params![record.revision as i64, record.authority.active(), kind, observed_at, rowid])?;
        rowid
    } else {
        connection.execute("INSERT INTO current_records(repository,id,revision,active,kind,observed_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![record.repository, record.id, record.revision as i64, record.authority.active(), kind, observed_at])?;
        connection.last_insert_rowid()
    };
    connection.execute(
        "INSERT INTO record_fts(rowid,title,body) VALUES(?1,?2,?3)",
        params![rowid, record.title, record.body],
    )?;
    connection.execute(
        "DELETE FROM edges WHERE repository=?1 AND source_id=?2",
        params![record.repository, record.id],
    )?;
    for link in &record.links {
        connection.execute("INSERT OR IGNORE INTO edges(repository,source_id,target_id,relation) VALUES(?1,?2,?3,?4)",
            params![record.repository, record.id, link.target_id, serde_json::to_string(&link.relation)?])?;
    }
    Ok(())
}

pub(crate) fn validate_chains(connection: &Connection) -> Result<()> {
    let gaps: i64 = connection.query_row(
        "SELECT count(*) FROM revisions r WHERE r.revision>1 AND NOT EXISTS
         (SELECT 1 FROM revisions p WHERE p.repository=r.repository AND p.id=r.id AND p.revision=r.revision-1)",
        [], |row| row.get(0))?;
    if gaps != 0 {
        return Err(LocalError::Invalid(
            "canonical revision chain has gaps".into(),
        ));
    }
    let mut statement = connection.prepare(
        "SELECT r.record,p.record FROM revisions r JOIN revisions p ON
         p.repository=r.repository AND p.id=r.id AND p.revision=r.revision-1",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let next: Record = serde_json::from_str(&row.get::<_, String>(0)?)?;
        let previous: Record = serde_json::from_str(&row.get::<_, String>(1)?)?;
        if next.source != previous.source
            || next.kind != previous.kind
            || next.created_at != previous.created_at
        {
            return Err(LocalError::Invalid(format!(
                "immutable provenance changed: {}",
                next.id
            )));
        }
    }
    Ok(())
}

pub(crate) fn search(
    connection: &Connection,
    query: SearchQuery,
    memories_only: bool,
) -> Result<Vec<SearchHit>> {
    files::validate_scope(&query.repository)?;
    if query.limit == 0 || query.limit > MAX_SEARCH_RESULTS || query.text.len() > 4096 {
        return Err(LocalError::Invalid(
            "search limit must be 1..100 and text at most 4096 bytes".into(),
        ));
    }
    let literal = query
        .text
        .split_whitespace()
        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ");
    if literal.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "SELECT r.record,CASE WHEN instr(lower(record_fts.title),lower(?5))>0 THEN 2.0 ELSE 1.0 END AS score FROM record_fts
         JOIN current_records c ON c.rowid=record_fts.rowid
         JOIN revisions r USING(repository,id,revision)
         WHERE record_fts MATCH ?1 AND c.repository=?2 AND (?3 OR c.active=1) AND (NOT ?6 OR c.kind!='\"event\"')
         AND (?3 OR ((json_type(r.record,'$.metadata.applicability.stale') IS NULL
           OR json_type(r.record,'$.metadata.applicability.stale') IN ('null','false'))
           AND json_extract(r.record,'$.metadata.applicability.branch') IS NULL
           AND json_extract(r.record,'$.metadata.applicability.path') IS NULL))
         ORDER BY score DESC,c.observed_at DESC,c.id LIMIT ?4")?;
    let rows = statement.query_map(
        params![
            literal,
            query.repository,
            query.include_inactive,
            query.limit as i64,
            query.text,
            memories_only
        ],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
    )?;
    rows.map(|row| {
        let (json, score) = row?;
        Ok(SearchHit {
            record: serde_json::from_str(&json)?,
            score,
            feedback: None,
        })
    })
    .collect()
}

pub(crate) fn status(connection: &Connection, repository: Option<&str>) -> Result<StoreStatus> {
    if let Some(repository) = repository {
        files::validate_scope(repository)?;
    }
    let (records, repositories, events): (usize, usize, usize) = connection.query_row(
        "SELECT count(*),count(DISTINCT repository),coalesce(sum(kind='\"event\"'),0) FROM current_records WHERE (?1 IS NULL OR repository=?1)",
        [repository], |row| Ok((read_count(row,0)?,read_count(row,1)?,read_count(row,2)?)))?;
    let revisions = connection.query_row(
        "SELECT count(*) FROM revisions WHERE (?1 IS NULL OR repository=?1)",
        [repository],
        |row| read_count(row, 0),
    )?;
    let links = connection.query_row(
        "SELECT count(*) FROM edges WHERE (?1 IS NULL OR repository=?1)",
        [repository],
        |row| read_count(row, 0),
    )?;
    Ok(StoreStatus {
        records,
        revisions,
        repositories,
        events,
        memories: records - events,
        links,
    })
}

pub(crate) fn rebuild(root: &Path, connection: &mut Connection) -> Result<ReindexReport> {
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    transaction.execute_batch("DELETE FROM record_fts; DELETE FROM edges; DELETE FROM current_records; DELETE FROM revisions; DELETE FROM audit_records; DELETE FROM audit_archived; DELETE FROM feedback_records;")?;
    let files = files::visit_records(root, |record| project(&transaction, &record))?;
    validate_chains(&transaction)?;
    crate::audit::rebuild(root, &transaction)?;
    crate::feedback::rebuild(root, &transaction)?;
    transaction.pragma_update(None, "user_version", 3)?;
    let state = status(&transaction, None)?;
    let dangling_links = transaction.query_row(
        "SELECT count(*) FROM edges e WHERE NOT EXISTS(SELECT 1 FROM current_records c WHERE c.repository=e.repository AND c.id=e.target_id)",
        [], |row| read_count(row,0))?;
    transaction.commit()?;
    Ok(ReindexReport {
        files,
        revisions: state.revisions,
        records: state.records,
        dangling_links,
    })
}

fn read_count(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<usize> {
    let count: i64 = row.get(index)?;
    usize::try_from(count).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}
