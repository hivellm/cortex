//! Read-only migration of daily SQLite evidence into canonical local revisions.
use super::{identity, ImportOptions, ImportReport};
use crate::{
    files, Authority, CaptureRequest, LocalError, LocalStore, Record, RecordKind, Relation, Result,
    RevisionRequest, Source, TypedLink,
};
use cortex_storage::daily_memory::{MemoryCategory, MemoryEvidence, MemoryRecord, MemoryScope};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};

const MAX_SOURCE_BYTES: usize = 128 * 1024 * 1024;
const MAX_PREPARED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Progress {
    mapping: String,
    snapshot: String,
    next: usize,
}

struct Version {
    request: CaptureRequest,
    version: u64,
    position: String,
}

fn invalid(message: impl Into<String>) -> LocalError {
    LocalError::Invalid(message.into())
}

/// The source connection never applies migrations or changes journal mode.
pub fn import_daily(
    store: &LocalStore,
    source: &Path,
    options: &ImportOptions,
) -> Result<ImportReport> {
    options.validate()?;
    files::ensure_no_symlinks(source)?;
    let source = fs::canonicalize(source)?;
    let source_name = source.to_string_lossy().into_owned();
    let mut report = ImportReport {
        source: source_name.clone(),
        files_scanned: 1,
        ..Default::default()
    };
    let mut connection = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    let transaction = connection.transaction()?;
    let loaded = load(&transaction, &source_name, options);
    let (versions, fingerprint, bytes) = match loaded {
        Ok(value) => value,
        Err(error) => {
            report.rejected = 1;
            report.failure(&source_name, None, error);
            return Ok(report);
        }
    };
    report.bytes_scanned = bytes as u64;
    transaction.commit()?;
    let state_dir = store.root().join("import-state");
    let key = files::hash(&files::canonical(&(
        "daily",
        &options.namespace,
        &source_name,
    ))?);
    let state_path = state_dir.join(format!("daily-{key}.json"));
    files::ensure_no_symlinks(&state_path)?;
    let _lock = if options.dry_run {
        None
    } else {
        files::ensure_no_symlinks(&state_dir)?;
        fs::create_dir_all(&state_dir)?;
        let path = state_dir.join(format!("daily-{key}.lock"));
        files::ensure_no_symlinks(&path)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let started = std::time::Instant::now();
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if started.elapsed() >= std::time::Duration::from_secs(30) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "daily import progress lock timed out",
                        )
                        .into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Some(file)
    };
    let mapping = options.mapping_hash()?;
    let mut progress = match fs::read(&state_path) {
        Ok(bytes) => serde_json::from_slice::<Progress>(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Progress {
            mapping: mapping.clone(),
            snapshot: fingerprint.clone(),
            next: 0,
        },
        Err(error) => return Err(error.into()),
    };
    if progress.mapping != mapping
        || progress.snapshot != fingerprint
        || progress.next > versions.len()
    {
        report.failure(&source_name, None, "daily source or repository mapping changed since import; use a separately reviewed namespace");
        report.rejected = 1;
        return Ok(report);
    }
    let destination = if options.dry_run {
        Some(store.readonly_index()?)
    } else {
        None
    };
    // Checkpoints accelerate publication; they never substitute for durable evidence.
    for version in versions.iter().take(progress.next) {
        let input = &version.request;
        let id = input
            .id
            .as_deref()
            .ok_or_else(|| invalid("missing import identity"))?;
        let existing = if let Some(connection) = &destination {
            crate::index::get(
                connection.connection(),
                &input.repository,
                id,
                Some(version.version),
            )?
        } else {
            store.get_revision(&input.repository, id, version.version)?
        };
        if !existing.as_ref().is_some_and(|record| same(record, input)) {
            report.rejected += 1;
            report.failure(
                &source_name,
                Some(version.position.clone()),
                "import checkpoint has missing or conflicting local revision",
            );
            return Ok(report);
        }
    }
    // A completed retry still validates the source fingerprint; it need not rewrite records.
    if progress.next == versions.len() {
        report.duplicates = versions.len();
        report.unchanged_files = 1;
        report.complete = true;
        return Ok(report);
    }
    let mut planned = std::collections::HashMap::new();
    for version in versions
        .iter()
        .skip(progress.next)
        .take(options.max_records)
    {
        report.records_seen += 1;
        if options.dry_run {
            if let Err(error) = preflight_version(
                destination
                    .as_ref()
                    .ok_or_else(|| invalid("missing dry-run snapshot"))?
                    .connection(),
                version,
                &planned,
            ) {
                report.rejected += 1;
                report.failure(&source_name, Some(version.position.clone()), error);
                break;
            }
            planned.insert(
                (
                    version.request.repository.clone(),
                    version.request.id.clone().unwrap_or_default(),
                ),
                version.version,
            );
        } else {
            match write_version(store, version) {
                Ok(true) => report.imported += 1,
                Ok(false) => report.duplicates += 1,
                Err(error) => {
                    report.rejected += 1;
                    report.failure(&source_name, Some(version.position.clone()), error);
                    break;
                }
            }
        }
        progress.next += 1;
        if !options.dry_run {
            let mut temporary = tempfile::NamedTempFile::new_in(&state_dir)?;
            temporary.write_all(&files::canonical(&progress)?)?;
            temporary.as_file().sync_all()?;
            temporary
                .persist(&state_path)
                .map_err(|error| error.error)?;
            files::sync_directory(&state_dir)?;
        }
    }
    report.limit_reached = progress.next < versions.len() && report.errors.is_empty();
    report.complete = progress.next == versions.len() && report.errors.is_empty();
    Ok(report)
}

fn check_schema(connection: &Connection, table: &str, columns: &[&str]) -> Result<()> {
    let mut query = connection.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let actual = query
        .query_map([table], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if columns
        .iter()
        .any(|column| !actual.iter().any(|item| item == column))
    {
        return Err(invalid(format!(
            "unsupported daily schema: {table} lacks required columns"
        )));
    }
    Ok(())
}

fn load(
    connection: &Connection,
    source: &str,
    options: &ImportOptions,
) -> Result<(Vec<Version>, String, usize)> {
    check_schema(connection, "daily_memory_records", &["scope", "id", "data"])?;
    check_schema(
        connection,
        "daily_memory_evidence",
        &["event_id", "scope", "memory_id", "version", "data"],
    )?;
    for table in ["daily_memory_records", "daily_memory_evidence"] {
        let sql = format!("SELECT coalesce(max(length(CAST(data AS BLOB))),0) FROM {table}");
        let maximum: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
        if maximum > crate::MAX_RECORD_BYTES as i64 {
            return Err(invalid(format!("daily source row exceeds 4 MiB: {table}")));
        }
    }
    let orphans: i64 = connection.query_row("SELECT count(*) FROM daily_memory_evidence e WHERE NOT EXISTS (SELECT 1 FROM daily_memory_records r WHERE r.scope=e.scope AND r.id=e.memory_id)", [], |row| row.get(0))?;
    if orphans != 0 {
        return Err(invalid("daily evidence contains orphaned history"));
    }
    let mut rows =
        connection.prepare("SELECT scope,id,data FROM daily_memory_records ORDER BY scope,id")?;
    let mut rows = rows.query([])?;
    let mut versions = Vec::new();
    let mut prepared_bytes = 0_usize;
    let mut identities = std::collections::HashSet::new();
    let mut event_ids = std::collections::HashSet::new();
    let mut digest = Sha256::new();
    let mut bytes = 0;
    while let Some(row) = rows.next()? {
        let scope: String = row.get(0)?;
        let id: String = row.get(1)?;
        let snapshot: String = row.get(2)?;
        if !identities.insert((scope.clone(), id.clone())) {
            return Err(invalid("duplicate daily snapshot identity"));
        }
        account(&mut bytes, &mut digest, &(&scope, &id, &snapshot))?;
        let memory: MemoryRecord = serde_json::from_str(&snapshot)?;
        let expected_scope = match &memory.scope {
            MemoryScope::Repository(repo) => format!("repo:{repo}"),
            MemoryScope::Global => "global".into(),
        };
        if scope != expected_scope
            || memory.id != id
            || memory.version == 0
            || memory.version > i64::MAX as u64
        {
            return Err(invalid(format!(
                "daily snapshot identity mismatch: {scope}/{id}"
            )));
        }
        let repository = match &memory.scope {
            MemoryScope::Repository(repo) => options.repository(Some(repo))?,
            MemoryScope::Global => options.global_repository.clone().ok_or_else(|| {
                invalid("global daily memory requires an explicit global_repository mapping")
            })?,
        };
        let local_id = identity("daily", &options.namespace, &scope, &id)?;
        let mut evidence = connection.prepare("SELECT event_id,version,data FROM daily_memory_evidence WHERE scope=?1 AND memory_id=?2 ORDER BY version")?;
        let mut evidence = evidence.query(rusqlite::params![scope, id])?;
        let mut first = None;
        let mut expected_version = 1;
        let start = versions.len();
        while let Some(row) = evidence.next()? {
            let event_id: String = row.get(0)?;
            let sql_version: i64 = row.get(1)?;
            let version = u64::try_from(sql_version)
                .map_err(|_| invalid("negative daily evidence version"))?;
            let raw: String = row.get(2)?;
            if !event_ids.insert(event_id.clone()) {
                return Err(invalid("duplicate daily evidence event identity"));
            }
            account(
                &mut bytes,
                &mut digest,
                &(&scope, &id, &event_id, version, &raw),
            )?;
            let value: MemoryEvidence = serde_json::from_str(&raw)?;
            if version != expected_version
                || value.version != version
                || value.event_id != event_id
                || value.payload.decision_id != id
                || value.kind != cortex_core::events::Kind::Decision
            {
                return Err(invalid(format!(
                    "noncontiguous or inconsistent daily evidence: {scope}/{id} version {version}"
                )));
            }
            if version > memory.version
                || value.capture_kind != memory.capture_kind
                || value.sources != memory.sources
            {
                return Err(invalid(
                    "daily evidence differs from snapshot provenance/version",
                ));
            }
            chrono::DateTime::parse_from_rfc3339(&value.occurred_at)
                .map_err(|e| invalid(e.to_string()))?;
            if version == 1
                && (event_id != memory.event_id
                    || value.occurred_at != memory.captured_at
                    || value.origin != memory.origin)
            {
                return Err(invalid(
                    "daily initial evidence differs from immutable snapshot origin",
                ));
            }
            let authority = match value.payload.status.as_str() {
                "proposed" => Authority::Proposed,
                "accepted" => Authority::Accepted,
                "superseded" => Authority::Superseded,
                "deprecated" => Authority::Deprecated,
                _ => return Err(invalid("unsupported daily status")),
            };
            let payload_hash = format!(
                "sha256:{}",
                files::hash(&serde_json::to_vec(&value.payload)?)
            );
            if version == memory.version {
                let status = serde_json::to_value(memory.status)?;
                let category = match memory.category {
                    MemoryCategory::Decision => "decision",
                    MemoryCategory::Lesson => "lesson",
                };
                let mut tags = vec!["daily-memory".to_owned(), category.to_owned()];
                if memory.stale {
                    tags.push("stale".into());
                }
                if value.payload.title != memory.title
                    || value.payload.body != memory.body
                    || status.as_str() != Some(value.payload.status.as_str())
                    || value.stale != memory.stale
                    || value.payload.supersedes != memory.supersedes
                    || payload_hash != memory.content_hash
                    || value.payload.tags != tags
                    || value.payload.cas_ref.is_some()
                {
                    return Err(invalid(
                        "latest daily evidence does not match current snapshot",
                    ));
                }
            }
            let source_record = first.get_or_insert_with(|| Source {
                kind: "daily-sqlite".into(), locator: source.into(), source_id: Some(memory.event_id.clone()),
                captured_at: Some(memory.captured_at.clone()), source_hash: Some(files::hash(raw.as_bytes())),
                payload: json!({"namespace":options.namespace,"original_scope":scope,"original_id":id,"initial_evidence":serde_json::from_str::<serde_json::Value>(&raw).expect("validated JSON"),"initial_evidence_raw":raw}),
            });
            let links = value
                .payload
                .supersedes
                .as_ref()
                .map(|prior| -> Result<TypedLink> {
                    Ok(TypedLink {
                        target_id: identity("daily", &options.namespace, &scope, prior)?,
                        relation: Relation::Supersedes,
                    })
                })
                .transpose()?
                .into_iter()
                .collect();
            let request = CaptureRequest {
                id: Some(local_id.clone()),
                repository: repository.clone(),
                kind: match memory.category {
                    MemoryCategory::Decision => RecordKind::Decision,
                    MemoryCategory::Lesson => RecordKind::Lesson,
                },
                authority,
                title: value.payload.title.clone(),
                body: value.payload.body.clone(),
                source: source_record.clone(),
                links,
                metadata: json!({"applicability":{"stale":value.stale,"branch":memory.branch,"path":memory.path,"pinned":memory.pinned},"daily_import":{"namespace":options.namespace,"original_scope":scope,"original_id":id,"source_version":version,"payload_hash":payload_hash,"evidence_sha256":files::hash(raw.as_bytes()),"evidence":value,"evidence_raw":raw,"snapshot_sha256":files::hash(snapshot.as_bytes()),"snapshot":memory,"snapshot_raw":snapshot}}),
            };
            LocalStore::validate_capture(&request)?;
            prepared_bytes = prepared_bytes
                .checked_add(files::canonical(&request)?.len())
                .ok_or_else(|| invalid("prepared daily snapshot size overflow"))?;
            if prepared_bytes > MAX_PREPARED_BYTES {
                return Err(invalid(
                    "prepared daily snapshot exceeds bounded 64 MiB import limit",
                ));
            }
            versions.push(Version {
                version,
                position: format!("{scope}/{id}@{version}"),
                request,
            });
            expected_version += 1;
        }
        if expected_version != memory.version + 1 || versions.len() == start {
            return Err(invalid(format!("missing daily history: {scope}/{id}")));
        }
    }
    Ok((versions, format!("{:x}", digest.finalize()), bytes))
}

fn account<T: Serialize>(bytes: &mut usize, digest: &mut Sha256, value: &T) -> Result<()> {
    let encoded = files::canonical(value)?;
    *bytes = bytes
        .checked_add(encoded.len())
        .ok_or_else(|| invalid("daily snapshot size overflow"))?;
    if *bytes > MAX_SOURCE_BYTES {
        return Err(invalid(
            "daily snapshot exceeds bounded 128 MiB import limit",
        ));
    }
    digest.update((encoded.len() as u64).to_le_bytes());
    digest.update(encoded);
    Ok(())
}

fn same(existing: &Record, request: &CaptureRequest) -> bool {
    existing.id == request.id.as_deref().unwrap_or("")
        && existing.repository == request.repository
        && existing.kind == request.kind
        && existing.authority == request.authority
        && existing.title == request.title
        && existing.body == request.body
        && existing.source == request.source
        && existing.links == request.links
        && existing.metadata == request.metadata
}

fn preflight_version(
    connection: &Connection,
    version: &Version,
    planned: &std::collections::HashMap<(String, String), u64>,
) -> Result<()> {
    let input = &version.request;
    let id = input
        .id
        .as_deref()
        .ok_or_else(|| invalid("missing import identity"))?;
    if let Some(existing) =
        crate::index::get(connection, &input.repository, id, Some(version.version))?
    {
        if !same(&existing, input) {
            return Err(LocalError::Conflict(format!(
                "imported revision conflicts: {}",
                version.position
            )));
        }
        return Ok(());
    }
    let current = crate::index::get(connection, &input.repository, id, None)?;
    if current
        .as_ref()
        .is_some_and(|record| record.revision >= version.version)
    {
        return Err(LocalError::Conflict(
            "imported revision sequence changed".into(),
        ));
    }
    if version.version == 1 {
        return Ok(());
    }
    if planned.get(&(input.repository.clone(), id.to_owned())) == Some(&(version.version - 1)) {
        return Ok(());
    }
    if current.as_ref().is_some_and(|record| {
        record.revision == version.version - 1 && record.source == input.source
    }) {
        return Ok(());
    }
    Err(LocalError::Conflict(
        "missing or conflicting imported predecessor".into(),
    ))
}

fn write_version(store: &LocalStore, version: &Version) -> Result<bool> {
    let input = &version.request;
    let id = input
        .id
        .as_deref()
        .ok_or_else(|| invalid("missing import identity"))?;
    if let Some(existing) = store.get_revision(&input.repository, id, version.version)? {
        if !same(&existing, input) {
            return Err(LocalError::Conflict(format!(
                "imported revision conflicts: {}",
                version.position
            )));
        }
        return Ok(false);
    }
    if version.version == 1 {
        return Ok(store.capture(input.clone())?.created);
    }
    let previous = store
        .get(&input.repository, id)?
        .ok_or_else(|| invalid("missing imported predecessor"))?;
    if previous.revision + 1 != version.version {
        return Err(LocalError::Conflict(
            "imported revision sequence changed".into(),
        ));
    }
    store.revise(
        &input.repository,
        id,
        previous.revision,
        RevisionRequest {
            title: input.title.clone(),
            body: input.body.clone(),
            authority: input.authority,
            links: input.links.clone(),
            metadata: input.metadata.clone(),
        },
    )?;
    Ok(true)
}
