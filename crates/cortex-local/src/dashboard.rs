//! Dashboard projections over current records, never sampled archive totals.
use crate::{files, model::*};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowseQuery {
    pub repository: String,
    #[serde(default)]
    pub kind: Option<RecordKind>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub include_inactive: bool,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowsePage {
    pub records: Vec<Record>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositorySummary {
    pub repository: String,
    pub records: u64,
    pub memories: u64,
    pub events: u64,
    pub links: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryInventory {
    pub repositories: Vec<RepositorySummary>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    repository: String,
    kind: Option<RecordKind>,
    include_inactive: bool,
    updated: i64,
    id: String,
}

pub(crate) fn repositories(connection: &Connection) -> Result<RepositoryInventory> {
    let mut statement = connection.prepare(
        "SELECT c.repository,count(*),sum(c.kind='\"event\"'),
         (SELECT count(*) FROM edges e WHERE e.repository=c.repository)
         FROM current_records c GROUP BY c.repository ORDER BY c.repository COLLATE BINARY",
    )?;
    let repositories = statement
        .query_map([], |row| {
            let count = |column| -> rusqlite::Result<u64> {
                let value: i64 = row.get(column)?;
                u64::try_from(value)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
            };
            let records = count(1)?;
            let events = count(2)?;
            Ok(RepositorySummary {
                repository: row.get(0)?,
                records,
                memories: records - events,
                events,
                links: count(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(RepositoryInventory { repositories })
}

pub(crate) fn browse(connection: &Connection, query: BrowseQuery) -> Result<BrowsePage> {
    files::validate_scope(&query.repository)?;
    if !(1..=100).contains(&query.limit) {
        return Err(LocalError::Invalid("browse limit must be 1..100".into()));
    }
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| {
            if value.len() > 8192 {
                return Err(LocalError::Invalid("invalid browse cursor".into()));
            }
            let cursor: Cursor = serde_json::from_str(value)
                .map_err(|_| LocalError::Invalid("invalid browse cursor".into()))?;
            if cursor.version != 1
                || cursor.repository != query.repository
                || cursor.kind != query.kind
                || cursor.include_inactive != query.include_inactive
            {
                return Err(LocalError::Invalid(
                    "browse cursor scope or filters changed".into(),
                ));
            }
            files::validate_id(&cursor.id)?;
            Ok(cursor)
        })
        .transpose()?;
    let kind = query
        .kind
        .map(|kind| serde_json::to_string(&kind))
        .transpose()?;
    // Millisecond UTC keys normalize imported RFC3339 offsets without altering records.
    let mut statement = connection.prepare(
        "WITH candidates AS (
           SELECT c.id,r.record,
             CAST(round((julianday(json_extract(r.record,'$.updated_at'))-2440587.5)*86400000) AS INTEGER) AS updated
           FROM current_records c JOIN revisions r USING(repository,id,revision)
           WHERE c.repository=?1 AND (?2 IS NULL OR c.kind=?2)
             AND (?3 OR (c.active=1
               AND (json_type(r.record,'$.metadata.applicability.stale') IS NULL OR json_type(r.record,'$.metadata.applicability.stale') IN ('null','false'))
               AND json_extract(r.record,'$.metadata.applicability.branch') IS NULL
               AND json_extract(r.record,'$.metadata.applicability.path') IS NULL)))
         SELECT record,updated FROM candidates
         WHERE (?4 IS NULL OR updated<?4 OR (updated=?4 AND id>?5))
         ORDER BY updated DESC,id ASC LIMIT ?6",
    )?;
    let rows = statement.query_map(
        params![
            query.repository,
            kind,
            query.include_inactive,
            cursor.as_ref().map(|cursor| cursor.updated),
            cursor.as_ref().map(|cursor| &cursor.id),
            (query.limit + 1) as i64
        ],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let mut records = Vec::new();
    let mut last_updated = 0;
    let mut has_more = false;
    for row in rows {
        let (record, updated) = row?;
        if records.len() == query.limit {
            has_more = true;
            break;
        }
        records.push(serde_json::from_str::<Record>(&record)?);
        last_updated = updated;
    }
    let next_cursor = if has_more {
        let last = records.last().expect("nonzero validated page limit");
        Some(serde_json::to_string(&Cursor {
            version: 1,
            repository: query.repository,
            kind: query.kind,
            include_inactive: query.include_inactive,
            updated: last_updated,
            id: last.id.clone(),
        })?)
    } else {
        None
    };
    Ok(BrowsePage {
        records,
        next_cursor,
    })
}
