//! Read-only migration of historical zstd NDJSON archives.
//!
//! Event identity uses namespace, original repository domain and event ID.
//! Identical events in multiple files retain the first canonical source; every
//! occurrence retains its locator and actual line digest in the derived import
//! ledger. Different original JSON under the same identity is a durable rejection.
//! The ledger is reconstructable by reimporting the unchanged source archives.
use super::{identity, ImportOptions, ImportReport};
use crate::{files, Authority, CaptureRequest, LocalError, LocalStore, RecordKind, Result, Source};
use cortex_storage::archive_tail::{TailCheckpoint, TailError, TailLimits, TailReader};
use fs2::FileExt;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_PENDING_BYTES: usize = 7 * 1024 * 1024;
const MAX_DISCOVERY_DIRECTORY: usize = 100_000;
const MAX_DISCOVERY_TOTAL: usize = 1_000_000;
const MAX_DISCOVERY_DEPTH: usize = 64;
const MAX_DRY_IDENTITIES: usize = 100_000;
type DryIdentities = HashMap<(String, String), (RecordKind, String)>;

#[derive(Clone)]
struct Snapshot {
    size: u64,
    hash: String,
}

#[derive(Clone)]
struct Progress {
    path: String,
    snapshot: Snapshot,
    generation: i64,
    checkpoint: TailCheckpoint,
    lines: u64,
    rejected: usize,
    finished: bool,
}

struct Item {
    path: String,
    generation: i64,
    line: u64,
    line_hash: String,
    original: Vec<u8>,
    candidate: std::result::Result<CaptureRequest, String>,
}

#[derive(Default)]
struct Pending {
    items: Vec<Item>,
    states: BTreeMap<String, Progress>,
    bytes: usize,
}

/// Import explicit source archives without mutating their bytes or metadata.
/// `records_seen` counts newly decoded lines, including rejected lines. Dry-run
/// validates candidates but reports zero imported records and writes no ledger.
/// `max_files` bounds decoded files; unchanged checkpoints are verified separately.
/// `bytes_scanned` sums source snapshot sizes, not repeated verification I/O.
/// Source hashing streams with bounded memory but has no wall-clock time budget.
pub fn import_archive(
    store: &LocalStore,
    source: &Path,
    options: &ImportOptions,
) -> Result<ImportReport> {
    options.validate()?;
    files::ensure_no_symlinks(source)?;
    let source = fs::canonicalize(source)?;
    let source_name = path_text(&source)?;
    let mut report = ImportReport {
        source: source_name.clone(),
        complete: true,
        ..Default::default()
    };
    let _lock = if options.dry_run {
        None
    } else {
        Some(import_lock(store.root())?)
    };
    let _dry_ledger = if options.dry_run {
        open_readonly_ledger(store.root(), options)?
    } else {
        None
    };
    let mut ledger = if options.dry_run {
        None
    } else {
        Some(open_ledger(store.root(), options)?)
    };
    let mut pending = Pending::default();
    let mut blocked = HashSet::new();
    let mut dry_identities = DryIdentities::new();
    let mut decoded_files = 0;
    let mut walk = Discovery::new(&source)?;
    while let Some(discovered) = walk.next() {
        let (path, excluded) = match discovered {
            Ok(item) => item,
            Err((path, error)) => {
                report.failure(&path.to_string_lossy(), None, error);
                report.complete = false;
                continue;
            }
        };
        if excluded {
            report.quarantined_files += 1;
            if archive_filename(&path) {
                report.quarantined_archives += 1;
            } else {
                report.quarantine_diagnostics += 1;
            }
            continue;
        }
        if report.records_seen >= options.max_records {
            report.limit_reached = true;
            report.complete = false;
            break;
        }
        report.files_scanned += 1;
        let path_name = match path_text(&path) {
            Ok(path) => path,
            Err(error) => {
                report.failure(&path.to_string_lossy(), None, error);
                report.complete = false;
                continue;
            }
        };
        let snapshot = match snapshot(&path) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                report.failure(&path_name, None, error);
                report.complete = false;
                continue;
            }
        };
        report.bytes_scanned = report.bytes_scanned.saturating_add(snapshot.size);
        let old = match ledger.as_ref().filter(|_| !options.dry_run) {
            Some(ledger) => load_progress(ledger, &options.namespace, &path_name)?,
            None => None,
        };
        let mut progress = Progress {
            path: path_name.clone(),
            snapshot: snapshot.clone(),
            generation: 1,
            checkpoint: TailCheckpoint::default(),
            lines: 0,
            rejected: 0,
            finished: false,
        };
        if let Some(old) = old {
            let same = old.snapshot.size == snapshot.size && old.snapshot.hash == snapshot.hash;
            let appended = if snapshot.size > old.snapshot.size {
                match hash_prefix(&path, old.snapshot.size) {
                    Ok(prefix) => prefix == old.snapshot.hash,
                    Err(error) => {
                        report.failure(&path_name, None, error);
                        report.complete = false;
                        continue;
                    }
                }
            } else {
                false
            };
            if same || appended {
                progress.generation = old.generation;
                progress.checkpoint = old.checkpoint;
                progress.lines = old.lines;
                progress.rejected = old.rejected;
                if old.rejected != 0 {
                    report.rejected += old.rejected;
                    report.failure(
                        &path_name,
                        None,
                        format!(
                            "{} previously rejected lines retained in import ledger",
                            old.rejected
                        ),
                    );
                    report.complete = false;
                }
                if same && old.finished {
                    report.unchanged_files += 1;
                    continue;
                }
            } else {
                progress.generation = old
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| LocalError::Invalid("source generation overflow".into()))?;
                report.failure(&path_name,None,format!("source replaced or rewritten; restarting verified generation {} from the beginning",progress.generation));
                report.complete = false;
            }
        }
        if decoded_files >= options.max_files {
            report.limit_reached = true;
            report.complete = false;
            break;
        }
        decoded_files += 1;
        if let Some(ledger) = &mut ledger {
            // Reserve the observed generation without advancing a single line.
            // A crash before a later checkpoint must still detect a rewrite of
            // the uncommitted source suffix and retain its occurrence provenance.
            let transaction = ledger.transaction_with_behavior(TransactionBehavior::Immediate)?;
            save_progress(
                &transaction,
                &options.namespace,
                &progress,
                progress.rejected as i64,
            )?;
            transaction.commit()?;
        }
        let mut reader = match TailReader::open_with_line_limit(
            &path,
            progress.checkpoint,
            crate::MAX_RECORD_BYTES,
        ) {
            Ok(reader) => reader,
            Err(error) => {
                report.failure(&path_name, None, error);
                report.complete = false;
                continue;
            }
        };
        loop {
            let remaining = options.max_records.saturating_sub(report.records_seen);
            if remaining == 0 {
                report.limit_reached = true;
                report.complete = false;
                break;
            }
            let batch = reader.poll(TailLimits {
                max_lines: remaining.min(256),
                max_line_bytes: crate::MAX_RECORD_BYTES,
                ..TailLimits::default()
            });
            let batch_count = batch.lines.len();
            for line in batch.lines {
                progress.lines += 1;
                report.records_seen += 1;
                let line_hash = files::hash(&line);
                let candidate =
                    candidate(&line, &path_name, progress.lines, options).and_then(|candidate| {
                        LocalStore::validate_capture(&candidate)?;
                        Ok(candidate)
                    });
                let estimated = match &candidate {
                    Ok(candidate) => files::canonical(candidate)?.len() + 512,
                    Err(_) => line.len() + 512,
                };
                if !pending.items.is_empty()
                    && (pending.items.len() >= 256 || pending.bytes + estimated > MAX_PENDING_BYTES)
                {
                    flush(
                        store,
                        ledger.as_mut(),
                        options,
                        &mut pending,
                        &mut blocked,
                        &mut report,
                        &mut dry_identities,
                    )?;
                }
                if candidate.is_err() {
                    progress.rejected += 1;
                }
                pending.bytes += estimated;
                pending.items.push(Item {
                    path: path_name.clone(),
                    generation: progress.generation,
                    line: progress.lines,
                    line_hash,
                    original: line,
                    candidate: candidate.map_err(|error| bounded(&error.to_string(), 2048)),
                });
            }
            progress.checkpoint = batch.checkpoint;
            let at_end = batch.eof
                || (batch.checkpoint.frame_start == snapshot.size
                    && batch.checkpoint.line_ordinal == 0
                    && !batch.partial);
            progress.finished = at_end && !batch.partial && batch.error.is_none();
            if !options.dry_run {
                pending.states.insert(path_name.clone(), progress.clone());
            }
            if pending.items.len() >= 256
                || pending.states.len() >= 256
                || pending.bytes >= MAX_PENDING_BYTES
            {
                flush(
                    store,
                    ledger.as_mut(),
                    options,
                    &mut pending,
                    &mut blocked,
                    &mut report,
                    &mut dry_identities,
                )?;
            }
            if blocked.contains(&path_name) {
                break;
            }
            if let Some(error) = batch.error {
                report.decoder_failed_files += 1;
                if matches!(error, TailError::LineTooLong { .. }) {
                    report.line_limit_files += 1;
                }
                report.failure(&path_name, Some(progress.lines.to_string()), error);
                report.complete = false;
                break;
            }
            if at_end {
                if batch.partial {
                    report.incomplete_files += 1;
                    let reason = if batch.frame_open
                        && batch.pending_line_bytes == 0
                        && !batch.replaying
                    {
                        report.open_frame_files += 1;
                        "open zstd frame at EOF; all complete lines recovered; append remains retryable"
                    } else {
                        if batch.pending_line_bytes != 0 {
                            report.partial_line_files += 1;
                            report.partial_line_bytes += batch.pending_line_bytes as u64;
                        }
                        "incomplete compressed frame or unterminated line; tail remains retryable"
                    };
                    report.failure(&path_name, Some(progress.lines.to_string()), reason);
                    report.complete = false;
                }
                break;
            }
            if batch_count == 0
                && batch.lines_processed == 0
                && batch.bytes_read == 0
                && batch.bytes_decoded == 0
            {
                report.failure(
                    &path_name,
                    Some(progress.lines.to_string()),
                    "archive decoder made no progress",
                );
                report.complete = false;
                break;
            }
        }
        if report.limit_reached {
            break;
        }
    }
    flush(
        store,
        ledger.as_mut(),
        options,
        &mut pending,
        &mut blocked,
        &mut report,
        &mut dry_identities,
    )?;
    Ok(report)
}

// Historical archives predate the current live bus envelope. Validate only the
// migration contract; preserve every original field separately as source evidence.
#[derive(Deserialize)]
struct ArchivedEnvelope {
    event_id: String,
    schema_version: String,
    occurred_at: String,
    session_id: String,
    #[serde(rename = "stream")]
    _stream: ArchivedStream,
    #[serde(rename = "tool")]
    _tool: String,
    kind: cortex_core::events::Kind,
    context: ArchivedContext,
    payload: Value,
    #[serde(default)]
    parent_event_id: Option<String>,
}

#[derive(Deserialize)]
enum ArchivedStream {
    #[serde(rename = "live", alias = "Live")]
    Live,
    #[serde(rename = "bootstrap", alias = "Bootstrap")]
    Bootstrap,
}

#[derive(Deserialize)]
struct ArchivedContext {
    #[serde(default)]
    repo: Option<String>,
}

fn candidate(
    line: &[u8],
    path: &str,
    number: u64,
    options: &ImportOptions,
) -> Result<CaptureRequest> {
    let text = std::str::from_utf8(line)
        .map_err(|error| LocalError::Invalid(format!("archive line is not UTF-8: {error}")))?;
    let original: Value = serde_json::from_str(text)?;
    let object = original
        .as_object()
        .ok_or_else(|| LocalError::Invalid("archive event must be a JSON object".into()))?;
    let missing_hash = !object.contains_key("content_hash");
    if object
        .get("content_hash")
        .is_some_and(|value| !value.is_string())
    {
        return Err(LocalError::Invalid(
            "legacy content_hash must be a string when present".into(),
        ));
    }
    let envelope: ArchivedEnvelope = serde_json::from_value(original.clone())?;
    if envelope.schema_version != "1" {
        return Err(LocalError::Invalid(
            "unsupported legacy envelope schema".into(),
        ));
    }
    files::validate_id(&envelope.event_id)?;
    files::validate_id(&envelope.session_id)?;
    chrono::DateTime::parse_from_rfc3339(&envelope.occurred_at)
        .map_err(|error| LocalError::Invalid(format!("legacy occurred_at: {error}")))?;
    let original_scope = match envelope.context.repo.as_deref() {
        Some(repository) => format!("repo:{}", serde_json::to_string(repository)?),
        None => "unscoped".to_owned(),
    };
    let repository = options.repository(envelope.context.repo.as_deref())?;
    let id = identity(
        "archive",
        &options.namespace,
        &original_scope,
        &envelope.event_id,
    )?;
    let links = if let Some(parent) = &envelope.parent_event_id {
        files::validate_id(parent)?;
        vec![crate::TypedLink {
            target_id: identity("archive", &options.namespace, &original_scope, parent)?,
            relation: crate::Relation::DerivedFrom,
        }]
    } else {
        Vec::new()
    };
    let kind = serde_json::to_value(envelope.kind)?
        .as_str()
        .unwrap_or("event")
        .to_owned();
    let title = bounded(&format!("{}: {}", kind, envelope.event_id), 16 * 1024);
    let mut body = String::new();
    readable(&envelope.payload, &mut body);
    if body.is_empty() {
        body = bounded(
            &serde_json::to_string_pretty(&envelope.payload)?,
            crate::MAX_BODY_BYTES,
        );
    }
    Ok(CaptureRequest {
        id: Some(id),
        repository,
        kind: RecordKind::Event,
        authority: Authority::Observation,
        title,
        body,
        source: Source {
            kind: "legacy_archive".into(),
            locator: format!("{path}#line={number}"),
            source_id: Some(envelope.event_id),
            captured_at: Some(envelope.occurred_at),
            source_hash: Some(files::hash(line)),
            payload: original,
        },
        links,
        metadata: json!({"import":{"namespace":options.namespace,"original_scope":original_scope,"legacy_missing_content_hash":missing_hash,"duplicate_policy":"first canonical source; all occurrences in archive import ledger"}}),
    })
}

fn readable(value: &Value, body: &mut String) {
    if body.len() >= crate::MAX_BODY_BYTES {
        return;
    }
    match value {
        Value::String(text) => {
            if !body.is_empty() && body.len() < crate::MAX_BODY_BYTES {
                body.push('\n');
            }
            body.push_str(&bounded(text, crate::MAX_BODY_BYTES - body.len()));
        }
        Value::Array(values) => {
            for value in values {
                readable(value, body);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if body.len() >= crate::MAX_BODY_BYTES {
                    break;
                }
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&bounded(key, crate::MAX_BODY_BYTES - body.len()));
                if body.len() < crate::MAX_BODY_BYTES {
                    body.push(':');
                }
                readable(value, body);
            }
        }
        other => {
            let scalar = other.to_string();
            body.push_str(&bounded(&scalar, crate::MAX_BODY_BYTES - body.len()));
        }
    }
}

fn bounded(text: &str, max: usize) -> String {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn flush(
    store: &LocalStore,
    ledger: Option<&mut Connection>,
    options: &ImportOptions,
    pending: &mut Pending,
    blocked: &mut HashSet<String>,
    report: &mut ImportReport,
    dry_identities: &mut DryIdentities,
) -> Result<()> {
    if pending.items.is_empty() && pending.states.is_empty() {
        return Ok(());
    }
    let mut requests = Vec::new();
    let mut staged: HashMap<(String, String), CaptureRequest> = HashMap::new();
    let mut lookup: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in &pending.items {
        if let Ok(candidate) = &item.candidate {
            if let Some(id) = &candidate.id {
                lookup
                    .entry(candidate.repository.clone())
                    .or_default()
                    .push(id.clone());
            }
        }
    }
    let mut existing = HashMap::new();
    for (repository, ids) in lookup {
        let records = lookup_records(store, &repository, &ids, options.dry_run)?;
        for (id, record) in ids.into_iter().zip(records) {
            if let Some(record) = record {
                existing.insert(
                    (repository.clone(), id),
                    (record.kind, record.source.payload),
                );
            }
        }
    }
    let mut outcomes: Vec<Option<String>> = Vec::with_capacity(pending.items.len());
    for item in &pending.items {
        let candidate = match &item.candidate {
            Ok(candidate) => candidate,
            Err(reason) => {
                outcomes.push(Some(reason.clone()));
                continue;
            }
        };
        let id = candidate
            .id
            .as_deref()
            .ok_or_else(|| LocalError::Invalid("import candidate lacks identity".into()))?;
        let key = (candidate.repository.clone(), id.to_owned());
        let original = if let Some(previous) = staged.get(&key) {
            Some((previous.kind, previous.source.payload.clone()))
        } else {
            existing.get(&key).cloned()
        };
        let dry_previous = dry_identities.get(&key);
        if original.is_none() && dry_previous.is_some() {
            let (kind, hash) = dry_previous
                .ok_or_else(|| LocalError::Invalid("missing dry-run identity".into()))?;
            if *kind == candidate.kind
                && *hash == files::hash(&files::canonical(&candidate.source.payload)?)
            {
                report.duplicates += 1;
                outcomes.push(None);
            } else {
                outcomes.push(Some(
                    "same archive identity has conflicting original JSON in dry-run".into(),
                ));
            }
        } else if let Some((kind, payload)) = original {
            if kind == RecordKind::Event && payload == candidate.source.payload {
                report.duplicates += 1;
                outcomes.push(None);
            } else {
                outcomes.push(Some("same archive event identity contains different original JSON; first canonical event preserved".into()));
            }
        } else {
            if options.dry_run {
                if dry_identities.len() >= MAX_DRY_IDENTITIES {
                    return Err(LocalError::Invalid(
                        "dry-run identity bound 100000 reached; reduce import scope".into(),
                    ));
                }
                dry_identities.insert(
                    key.clone(),
                    (
                        candidate.kind,
                        files::hash(&files::canonical(&candidate.source.payload)?),
                    ),
                );
            }
            staged.insert(key, candidate.clone());
            requests.push(candidate.clone());
            outcomes.push(None);
        }
    }
    if !requests.is_empty() && !options.dry_run {
        let result = store.capture_batch(requests)?;
        report.imported += result.created;
        report.duplicates += result.duplicates;
    }
    if options.dry_run {
        for (item, rejection) in pending.items.iter().zip(&outcomes) {
            if let Some(reason) = rejection {
                report.rejected += 1;
                report.failure(&item.path, Some(item.line.to_string()), reason);
                report.complete = false;
            }
        }
        pending.items.clear();
        pending.states.clear();
        pending.bytes = 0;
        return Ok(());
    }
    let ledger =
        ledger.ok_or_else(|| LocalError::Invalid("missing archive import ledger".into()))?;
    for state in pending.states.values() {
        if blocked.contains(&state.path) {
            continue;
        }
        match snapshot(Path::new(&state.path)) {
            Ok(current)
                if current.size == state.snapshot.size && current.hash == state.snapshot.hash => {}
            Ok(_) => {
                blocked.insert(state.path.clone());
                report.failure(
                    &state.path,
                    None,
                    "source changed while decoding; checkpoint not advanced; retry source",
                );
                report.complete = false;
            }
            Err(error) => {
                blocked.insert(state.path.clone());
                report.failure(&state.path, None, error);
                report.complete = false;
            }
        }
    }
    let transaction = ledger.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (item, rejection) in pending.items.iter().zip(&outcomes) {
        let record_id = item
            .candidate
            .as_ref()
            .ok()
            .and_then(|request| request.id.as_deref());
        let repository = item
            .candidate
            .as_ref()
            .ok()
            .map(|request| request.repository.as_str());
        transaction.execute(
            "INSERT OR REPLACE INTO occurrences(namespace,path,generation,line,source_hash,record_id,repository,rejection,raw_rejected) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![options.namespace,item.path,item.generation,item.line as i64,item.line_hash,record_id,repository,rejection,rejection.as_ref().map(|_| item.original.as_slice())])?;
        if let Some(reason) = rejection {
            report.rejected += 1;
            report.failure(&item.path, Some(item.line.to_string()), reason);
            report.complete = false;
        }
    }
    for state in pending.states.values() {
        if blocked.contains(&state.path) {
            continue;
        }
        let rejected: i64 = transaction.query_row(
            "SELECT count(*) FROM occurrences WHERE namespace=?1 AND path=?2 AND generation=?3 AND line<=?4 AND rejection IS NOT NULL",
            params![options.namespace,state.path,state.generation,state.lines as i64],|row| row.get(0))?;
        save_progress(&transaction, &options.namespace, state, rejected)?;
    }
    transaction.commit()?;
    pending.items.clear();
    pending.states.clear();
    pending.bytes = 0;
    Ok(())
}

fn save_progress(
    connection: &Connection,
    namespace: &str,
    state: &Progress,
    rejected: i64,
) -> Result<()> {
    connection.execute(
        "INSERT INTO source_generations(namespace,path,generation,size,hash) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(namespace,path,generation) DO UPDATE SET size=excluded.size,hash=excluded.hash",
        params![namespace,state.path,state.generation,state.snapshot.size as i64,state.snapshot.hash])?;
    connection.execute(
        "INSERT INTO sources(namespace,path,size,hash,generation,frame_start,line_ordinal,lines,rejected,finished) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(namespace,path) DO UPDATE SET size=excluded.size,hash=excluded.hash,generation=excluded.generation,frame_start=excluded.frame_start,line_ordinal=excluded.line_ordinal,lines=excluded.lines,rejected=excluded.rejected,finished=excluded.finished",
        params![namespace,state.path,state.snapshot.size as i64,state.snapshot.hash,state.generation,state.checkpoint.frame_start as i64,state.checkpoint.line_ordinal as i64,state.lines as i64,rejected,state.finished])?;
    Ok(())
}

fn import_lock(root: &Path) -> Result<File> {
    let path = root.join("archive-import.lock");
    files::ensure_no_symlinks(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let start = Instant::now();
    loop {
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
                if start.elapsed() >= Duration::from_secs(30) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "archive import lock timed out",
                    )
                    .into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn lookup_records(
    store: &LocalStore,
    repository: &str,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<Option<crate::Record>>> {
    if !dry_run {
        return store.get_many(repository, ids);
    }
    let snapshot = store.readonly_index()?;
    ids.iter()
        .map(|id| crate::index::get(snapshot.connection(), repository, id, None))
        .collect()
}

fn open_readonly_ledger(
    root: &Path,
    options: &ImportOptions,
) -> Result<Option<crate::readonly::ReadonlyDatabase>> {
    let path = root.join("archive-import.sqlite3");
    files::ensure_no_symlinks(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let snapshot = crate::readonly::ReadonlyDatabase::open(
        &path,
        &root.join("archive-import.lock"),
        None,
        Duration::from_secs(30),
    )?;
    let mapping: Option<String> = snapshot
        .connection()
        .query_row(
            "SELECT mapping_hash FROM namespaces WHERE namespace=?1",
            [&options.namespace],
            |row| row.get(0),
        )
        .optional()?;
    let expected_mapping = options.mapping_hash()?;
    if mapping.is_some_and(|mapping| mapping != expected_mapping) {
        return Err(LocalError::Conflict(
            "archive namespace mapping changed; use a new explicit namespace".into(),
        ));
    }
    Ok(Some(snapshot))
}

fn open_ledger(root: &Path, options: &ImportOptions) -> Result<Connection> {
    for name in [
        "archive-import.sqlite3",
        "archive-import.sqlite3-wal",
        "archive-import.sqlite3-shm",
    ] {
        files::ensure_no_symlinks(&root.join(name))?;
    }
    let connection = Connection::open(root.join("archive-import.sqlite3"))?;
    connection.busy_timeout(Duration::from_secs(30))?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > 1 {
        return Err(LocalError::Invalid(
            "unsupported archive import ledger schema".into(),
        ));
    }
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS namespaces(namespace TEXT PRIMARY KEY,mapping_hash TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS sources(namespace TEXT NOT NULL,path TEXT NOT NULL,size INTEGER NOT NULL,hash TEXT NOT NULL,generation INTEGER NOT NULL,frame_start INTEGER NOT NULL,line_ordinal INTEGER NOT NULL,lines INTEGER NOT NULL,rejected INTEGER NOT NULL,finished INTEGER NOT NULL,PRIMARY KEY(namespace,path));
        CREATE TABLE IF NOT EXISTS source_generations(namespace TEXT NOT NULL,path TEXT NOT NULL,generation INTEGER NOT NULL,size INTEGER NOT NULL,hash TEXT NOT NULL,PRIMARY KEY(namespace,path,generation));
        CREATE TABLE IF NOT EXISTS occurrences(namespace TEXT NOT NULL,path TEXT NOT NULL,generation INTEGER NOT NULL,line INTEGER NOT NULL,source_hash TEXT NOT NULL,record_id TEXT,repository TEXT,rejection TEXT,raw_rejected BLOB,PRIMARY KEY(namespace,path,generation,line));
        PRAGMA user_version=1;")?;
    let mapping = options.mapping_hash()?;
    let previous: Option<String> = connection
        .query_row(
            "SELECT mapping_hash FROM namespaces WHERE namespace=?1",
            [&options.namespace],
            |row| row.get(0),
        )
        .optional()?;
    if previous
        .as_ref()
        .is_some_and(|previous| previous != &mapping)
    {
        return Err(LocalError::Conflict(
            "archive namespace mapping changed; use a new explicit namespace".into(),
        ));
    }
    connection.execute(
        "INSERT OR IGNORE INTO namespaces(namespace,mapping_hash) VALUES(?1,?2)",
        params![options.namespace, mapping],
    )?;
    Ok(connection)
}

fn load_progress(connection: &Connection, namespace: &str, path: &str) -> Result<Option<Progress>> {
    type ProgressRow = (i64, String, i64, i64, i64, i64, i64, bool);
    let row: Option<ProgressRow> = connection.query_row(
        "SELECT size,hash,generation,frame_start,line_ordinal,lines,rejected,finished FROM sources WHERE namespace=?1 AND path=?2",
        params![namespace,path],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?))).optional()?;
    row.map(
        |(size, hash, generation, frame_start, line_ordinal, lines, rejected, finished)| {
            if [size, frame_start, line_ordinal, lines, rejected]
                .iter()
                .any(|value| *value < 0)
                || generation < 1
                || frame_start > size
            {
                return Err(LocalError::Invalid("invalid archive checkpoint".into()));
            }
            Ok(Progress {
                path: path.into(),
                snapshot: Snapshot {
                    size: size as u64,
                    hash,
                },
                generation,
                checkpoint: TailCheckpoint {
                    frame_start: frame_start as u64,
                    line_ordinal: line_ordinal as u64,
                },
                lines: lines as u64,
                rejected: rejected as usize,
                finished,
            })
        },
    )
    .transpose()
}

fn snapshot(path: &Path) -> Result<Snapshot> {
    files::ensure_no_symlinks(path)?;
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > i64::MAX as u64 {
        return Err(LocalError::Invalid(
            "archive source must be a bounded regular file".into(),
        ));
    }
    Ok(Snapshot {
        size: metadata.len(),
        hash: hash_prefix(path, metadata.len())?,
    })
}

fn hash_prefix(path: &Path, size: u64) -> Result<String> {
    let mut reader = File::open(path)?.take(size);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut consumed = 0u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        consumed += count as u64;
    }
    if consumed != size {
        return Err(LocalError::Invalid("source truncated while hashing".into()));
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| LocalError::Invalid("source path is not UTF-8".into()))
}

struct Discovery {
    stack: Vec<(std::vec::IntoIter<PathBuf>, bool)>,
    visited: usize,
}

fn archive_filename(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.ends_with(".parquet") || name.ends_with(".jsonl.zst") || name.ends_with(".ndjson.zst")
    })
}

fn redirected(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

impl Discovery {
    fn new(root: &Path) -> Result<Self> {
        let excluded = root.ancestors().any(|path| {
            path.file_name()
                .is_some_and(|name| name == "_quarantine" || name == "quarantine")
        });
        Ok(Self {
            stack: vec![(vec![root.to_owned()].into_iter(), excluded)],
            visited: 0,
        })
    }

    fn next(&mut self) -> Option<std::result::Result<(PathBuf, bool), (PathBuf, String)>> {
        loop {
            let (entries, excluded) = self.stack.last_mut()?;
            let inherited = *excluded;
            let Some(path) = entries.next() else {
                self.stack.pop();
                continue;
            };
            self.visited += 1;
            if self.visited > MAX_DISCOVERY_TOTAL {
                self.stack.clear();
                return Some(Err((
                    path,
                    "archive discovery exceeds 1000000 entries per invocation".into(),
                )));
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => return Some(Err((path, error.to_string()))),
            };
            if redirected(&metadata) {
                return Some(Ok((path, true)));
            }
            let excluded = inherited
                || path
                    .file_name()
                    .is_some_and(|name| name == "_quarantine" || name == "quarantine");
            if metadata.is_dir() {
                if self.stack.len() >= MAX_DISCOVERY_DEPTH {
                    return Some(Err((path, "archive discovery exceeds depth64".into())));
                }
                let entries = match fs::read_dir(&path).and_then(|entries| {
                    entries
                        .take(MAX_DISCOVERY_DIRECTORY + 1)
                        .map(|entry| entry.map(|entry| entry.path()))
                        .collect::<std::io::Result<Vec<_>>>()
                }) {
                    Ok(mut entries) => {
                        if entries.len() > MAX_DISCOVERY_DIRECTORY {
                            return Some(Err((
                                path,
                                "archive directory exceeds 100000 entries; narrow source directory"
                                    .into(),
                            )));
                        }
                        entries.sort();
                        entries
                    }
                    Err(error) => return Some(Err((path, error.to_string()))),
                };
                self.stack.push((entries.into_iter(), excluded));
                continue;
            }
            if !metadata.is_file() {
                return Some(Ok((path, true)));
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default();
            if excluded || name.ends_with(".reason") {
                return Some(Ok((path, true)));
            }
            if archive_filename(&path) {
                return Some(Ok((path, false)));
            }
        }
    }
}
