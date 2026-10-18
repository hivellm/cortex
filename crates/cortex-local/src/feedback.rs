//! Immutable declared feedback; eligibility is derived, never persisted authority.
use crate::{audit, files, index, *};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_FEEDBACK_BYTES: usize = 64 * 1024;
pub const MAX_FEEDBACK_EVIDENCE: usize = 10_000;
pub const LEGACY_FEEDBACK_REPOSITORY: &str = "legacy-feedback-unattributed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackEvidence {
    Explicit,
    Inferred,
    LegacyUnknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackJudgment {
    Helpful,
    Unhelpful,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackEligibility {
    Eligible,
    LegacyUnknown,
    Inferred,
    UnknownJudgment,
    Bundle,
    InconsistentQueryHashes,
    ConflictingJudgments,
    EvidenceLimit,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackSource {
    pub kind: String,
    pub reference: String,
    pub actor: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackRequest {
    pub id: String,
    pub repository: String,
    pub source: FeedbackSource,
    pub evidence: FeedbackEvidence,
    pub judgment: FeedbackJudgment,
    pub query_id: Option<String>,
    pub delivery_id: Option<String>,
    pub reference: Option<AuditReference>,
    pub comment: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackRecord {
    pub schema_version: u32,
    pub request: FeedbackRequest,
    pub recorded_at: String,
    pub legacy: Option<Value>,
    pub content_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackEntry {
    pub record: FeedbackRecord,
    pub eligibility: FeedbackEligibility,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackWriteOutcome {
    pub entry: FeedbackEntry,
    pub created: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedbackListQuery {
    pub repository: String,
    pub limit: usize,
    pub cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackListPage {
    pub entries: Vec<FeedbackEntry>,
    pub next_cursor: Option<String>,
}

fn invalid(message: &str) -> LocalError {
    LocalError::Invalid(format!("feedback: {message}"))
}
fn bounded_text(text: &str, max: usize) -> Result<()> {
    if text.trim().is_empty() || text.len() > max || text.chars().any(char::is_control) {
        return Err(invalid("source must be nonempty bounded text"));
    }
    Ok(())
}
fn timestamp(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("invalid UTC timestamp"))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(invalid("timestamp must be UTC"));
    }
    Ok(parsed)
}
fn sortable_timestamp(value: &str) -> Result<String> {
    Ok(timestamp(value)?.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
}
fn validate_common(request: &FeedbackRequest) -> Result<()> {
    files::validate_scope(&request.repository)?;
    audit::bounded_id(&request.id)?;
    bounded_text(&request.source.kind, 128)?;
    bounded_text(&request.source.reference, 2048)?;
    bounded_text(&request.source.actor, 256)?;
    if request
        .comment
        .as_ref()
        .is_some_and(|comment| comment.len() > 4096)
    {
        return Err(invalid("comment exceeds 4096 bytes"));
    }
    Ok(())
}

pub(crate) fn validate_legacy_request(request: &FeedbackRequest, legacy: &Value) -> Result<()> {
    validate_common(request)?;
    if request.repository != LEGACY_FEEDBACK_REPOSITORY
        || request.evidence != FeedbackEvidence::LegacyUnknown
        || request.query_id.is_some()
        || request.delivery_id.is_some()
        || request.reference.is_some()
        || request.judgment != FeedbackJudgment::Unknown
        || request.comment.is_some()
    {
        return Err(invalid("legacy feedback cannot acquire online attribution"));
    }
    let object = legacy
        .as_object()
        .ok_or_else(|| invalid("legacy payload must be object"))?;
    if object.len() != 4 {
        return Err(invalid("legacy payload fields mismatch"));
    }
    let path = legacy["source_path"]
        .as_str()
        .ok_or_else(|| invalid("legacy source path missing"))?;
    if request.source.kind != "legacy_sqlite"
        || request.source.reference != path
        || request.source.actor != "unknown"
        || legacy["table"] != "pre_thinking_feedback"
    {
        return Err(invalid("legacy source metadata mismatch"));
    }
    let raw = &legacy["raw_row"];
    let row = raw
        .as_object()
        .ok_or_else(|| invalid("legacy raw row must be object"))?;
    let fields = [
        "query_id",
        "intent",
        "helpful",
        "files_cited",
        "rating",
        "free_text",
        "implicit_score",
        "recorded_at",
    ];
    if row.len() != fields.len() || fields.iter().any(|field| !row.contains_key(*field)) {
        return Err(invalid("legacy row columns mismatch"));
    }
    for value in row.values() {
        let tagged = value
            .as_object()
            .ok_or_else(|| invalid("legacy value must retain storage class"))?;
        if tagged.len() != 2
            || !tagged.contains_key("storage_class")
            || !tagged.contains_key("value")
        {
            return Err(invalid("legacy storage fields mismatch"));
        }
        let valid = match value["storage_class"].as_str() {
            Some("null") => value["value"].is_null(),
            Some("integer") => value["value"]
                .as_str()
                .is_some_and(|s| s.parse::<i64>().is_ok_and(|n| n.to_string() == s)),
            Some("real") => value["value"]
                .as_str()
                .is_some_and(|s| s.len() == 16 && lowercase_hex(s)),
            Some("text" | "blob") => value["value"]
                .as_str()
                .is_some_and(|s| s.len() % 2 == 0 && lowercase_hex(s)),
            _ => false,
        };
        if !valid {
            return Err(invalid("invalid lossless legacy value"));
        }
    }
    if legacy["raw_row_sha256"].as_str() != Some(files::hash(&files::canonical(raw)?).as_str()) {
        return Err(invalid("legacy raw row hash mismatch"));
    }
    let expected = format!(
        "legacy-feedback-{}",
        files::hash(&files::canonical(&(path, "pre_thinking_feedback", raw))?)
    );
    if request.id != expected {
        return Err(invalid("legacy occurrence identity mismatch"));
    }
    let preview = FeedbackRecord {
        schema_version: 1,
        request: request.clone(),
        recorded_at: "9999-12-31T23:59:59.999999999+00:00".into(),
        legacy: Some(legacy.clone()),
        content_sha256: "0".repeat(64),
    };
    if files::canonical(&preview)?.len() > MAX_FEEDBACK_BYTES {
        return Err(invalid("canonical feedback exceeds 64 KiB"));
    }
    Ok(())
}
fn lowercase_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn validate_mode(request: &FeedbackRequest, legacy: Option<&Value>) -> Result<()> {
    validate_common(request)?;
    if let Some(legacy) = legacy {
        return validate_legacy_request(request, legacy);
    }
    if request.repository == LEGACY_FEEDBACK_REPOSITORY
        || request.evidence == FeedbackEvidence::LegacyUnknown
        || request.source.kind == "legacy_sqlite"
    {
        return Err(invalid("legacy feedback is import-only"));
    }
    Ok(())
}

fn validate_request(
    connection: &Connection,
    request: &FeedbackRequest,
    legacy: Option<&Value>,
    recorded_at: &str,
) -> Result<()> {
    validate_mode(request, legacy)?;
    if let Some(legacy) = legacy {
        return validate_legacy_request(request, legacy);
    }
    let query = request
        .query_id
        .as_deref()
        .ok_or_else(|| invalid("query ID required"))?;
    let delivery = request
        .delivery_id
        .as_deref()
        .ok_or_else(|| invalid("delivery ID required"))?;
    audit::bounded_id(query)?;
    audit::bounded_id(delivery)?;
    let receipt = audit::get(connection, &request.repository, delivery)?
        .ok_or_else(|| invalid("missing exact scoped delivery"))?;
    let AuditPayload::Delivery(draft) = receipt.payload else {
        return Err(invalid("feedback target is not delivery"));
    };
    if draft.query_id != query {
        return Err(invalid("query ID differs from delivery"));
    }
    if timestamp(recorded_at)? < timestamp(&receipt.recorded_at)? {
        return Err(invalid("feedback predates delivery"));
    }
    if let Some(reference) = &request.reference {
        if draft.outcome != PreparationOutcome::Prepared || !draft.references.contains(reference) {
            return Err(invalid("reference was not rendered by prepared delivery"));
        }
        if reference.repository != request.repository {
            return Err(invalid("reference crosses repository scope"));
        }
        let historical = index::get(
            connection,
            &request.repository,
            &reference.id,
            Some(reference.revision),
        )?
        .ok_or_else(|| invalid("missing historical reference"))?;
        if historical.kind == RecordKind::Event
            || AuditReference::from_record(&historical, reference.body_clipped)? != *reference
        {
            return Err(invalid("historical reference fingerprint mismatch"));
        }
    }
    Ok(())
}
pub(crate) fn make(
    connection: &Connection,
    request: FeedbackRequest,
    legacy: Option<Value>,
) -> Result<FeedbackRecord> {
    let mut record = FeedbackRecord {
        schema_version: 1,
        request,
        recorded_at: chrono::Utc::now().to_rfc3339(),
        legacy,
        content_sha256: String::new(),
    };
    validate_request(
        connection,
        &record.request,
        record.legacy.as_ref(),
        &record.recorded_at,
    )?;
    record.content_sha256 = files::hash(&files::canonical(&record)?);
    validate(connection, &record)?;
    Ok(record)
}
pub(crate) fn validate(connection: &Connection, record: &FeedbackRecord) -> Result<()> {
    timestamp(&record.recorded_at)?;
    if record.schema_version != 1 || files::canonical(record)?.len() > MAX_FEEDBACK_BYTES {
        return Err(invalid("unsupported schema or feedback size"));
    }
    let mut unsigned = record.clone();
    unsigned.content_sha256.clear();
    if files::hash(&files::canonical(&unsigned)?) != record.content_sha256 {
        return Err(invalid("feedback content hash mismatch"));
    }
    validate_request(
        connection,
        &record.request,
        record.legacy.as_ref(),
        &record.recorded_at,
    )
}
pub(crate) fn path(root: &Path, record: &FeedbackRecord) -> PathBuf {
    root.join("feedback")
        .join(files::hash(record.request.repository.as_bytes()))
        .join(format!(
            "{}.json",
            files::hash(record.request.id.as_bytes())
        ))
}
pub(crate) fn get(
    connection: &Connection,
    repository: &str,
    id: &str,
) -> Result<Option<FeedbackRecord>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT record FROM feedback_records WHERE repository=?1 AND id=?2",
            params![repository, id],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|json| serde_json::from_str(&json).map_err(LocalError::from))
        .transpose()
}
pub(crate) fn project(connection: &Connection, record: &FeedbackRecord) -> Result<()> {
    connection.execute(
        "INSERT INTO feedback_records(repository,id,recorded_at,record) VALUES(?1,?2,?3,?4)",
        params![
            record.request.repository,
            record.request.id,
            sortable_timestamp(&record.recorded_at)?,
            serde_json::to_string(record)?
        ],
    )?;
    Ok(())
}
fn basic_eligibility(request: &FeedbackRequest) -> FeedbackEligibility {
    if request.evidence == FeedbackEvidence::LegacyUnknown {
        FeedbackEligibility::LegacyUnknown
    } else if request.evidence == FeedbackEvidence::Inferred {
        FeedbackEligibility::Inferred
    } else if request.judgment == FeedbackJudgment::Unknown {
        FeedbackEligibility::UnknownJudgment
    } else if request.reference.is_none() {
        FeedbackEligibility::Bundle
    } else {
        FeedbackEligibility::Eligible
    }
}
fn group_key(request: &FeedbackRequest) -> Result<String> {
    let reference = request
        .reference
        .as_ref()
        .ok_or_else(|| invalid("group lacks reference"))?;
    String::from_utf8(files::canonical(&(
        request.repository.as_str(),
        request.query_id.as_deref(),
        reference.id.as_str(),
        reference.revision,
        reference.content_sha256.as_str(),
        reference.source_sha256.as_str(),
    ))?)
    .map_err(|_| invalid("invalid group encoding"))
}
pub(crate) struct EligibilityContext {
    limit: bool,
    inconsistent: HashSet<String>,
    conflicts: HashSet<String>,
}
impl EligibilityContext {
    pub(crate) fn load(connection: &Connection, repository: &str) -> Result<Self> {
        let mut context = Self {
            limit: false,
            inconsistent: HashSet::new(),
            conflicts: HashSet::new(),
        };
        if repository == LEGACY_FEEDBACK_REPOSITORY {
            return Ok(context);
        }
        let feedback_count: i64 = connection.query_row(
            "SELECT count(*) FROM feedback_records WHERE repository=?1",
            [repository],
            |r| r.get(0),
        )?;
        let delivery_count:i64=connection.query_row("SELECT count(*) FROM audit_records WHERE repository=?1 AND json_extract(record,'$.payload.type')='delivery'",[repository],|r|r.get(0))?;
        if feedback_count > MAX_FEEDBACK_EVIDENCE as i64
            || delivery_count > MAX_FEEDBACK_EVIDENCE as i64
        {
            context.limit = true;
            return Ok(context);
        }
        let mut hashes: HashMap<String, String> = HashMap::new();
        let mut statement=connection.prepare("SELECT record FROM audit_records WHERE repository=?1 AND json_extract(record,'$.payload.type')='delivery'")?;
        for row in statement.query_map([repository], |r| r.get::<_, String>(0))? {
            let record: AuditRecord = serde_json::from_str(&row?)?;
            if let AuditPayload::Delivery(draft) = record.payload {
                if hashes
                    .get(&draft.query_id)
                    .is_some_and(|hash| hash != &draft.query_sha256)
                {
                    context.inconsistent.insert(draft.query_id.clone());
                }
                hashes.insert(draft.query_id, draft.query_sha256);
            }
        }
        let mut judgments: HashMap<String, FeedbackJudgment> = HashMap::new();
        let mut statement =
            connection.prepare("SELECT record FROM feedback_records WHERE repository=?1")?;
        for row in statement.query_map([repository], |r| r.get::<_, String>(0))? {
            let record: FeedbackRecord = serde_json::from_str(&row?)?;
            if basic_eligibility(&record.request) != FeedbackEligibility::Eligible {
                continue;
            }
            let key = group_key(&record.request)?;
            if judgments
                .get(&key)
                .is_some_and(|judgment| *judgment != record.request.judgment)
            {
                context.conflicts.insert(key.clone());
            }
            judgments.insert(key, record.request.judgment);
        }
        Ok(context)
    }
    pub(crate) fn entry(&self, record: FeedbackRecord) -> Result<FeedbackEntry> {
        let mut eligibility = basic_eligibility(&record.request);
        if eligibility == FeedbackEligibility::Eligible {
            eligibility = if self.limit {
                FeedbackEligibility::EvidenceLimit
            } else if record
                .request
                .query_id
                .as_ref()
                .is_some_and(|query| self.inconsistent.contains(query))
            {
                FeedbackEligibility::InconsistentQueryHashes
            } else if self.conflicts.contains(&group_key(&record.request)?) {
                FeedbackEligibility::ConflictingJudgments
            } else {
                FeedbackEligibility::Eligible
            };
        }
        Ok(FeedbackEntry {
            record,
            eligibility,
        })
    }
}
pub(crate) fn list(connection: &Connection, query: &FeedbackListQuery) -> Result<FeedbackListPage> {
    files::validate_scope(&query.repository)?;
    if !(1..=100).contains(&query.limit) {
        return Err(invalid("list limit must be 1..100"));
    }
    let cursor_time = if let Some(id) = &query.cursor {
        audit::bounded_id(id)?;
        Some(sortable_timestamp(
            &get(connection, &query.repository, id)?
                .ok_or_else(|| invalid("cursor missing in scope"))?
                .recorded_at,
        )?)
    } else {
        None
    };
    let context = EligibilityContext::load(connection, &query.repository)?;
    let mut statement=connection.prepare("SELECT record FROM feedback_records WHERE repository=?1 AND (?2 IS NULL OR recorded_at<?2 OR (recorded_at=?2 AND id<?3)) ORDER BY recorded_at DESC,id DESC LIMIT ?4")?;
    let mut entries = Vec::new();
    for row in statement.query_map(
        params![
            query.repository,
            cursor_time,
            query.cursor,
            (query.limit + 1) as i64
        ],
        |r| r.get::<_, String>(0),
    )? {
        entries.push(context.entry(serde_json::from_str(&row?)?)?);
    }
    let next_cursor = if entries.len() > query.limit {
        entries.truncate(query.limit);
        entries.last().map(|entry| entry.record.request.id.clone())
    } else {
        None
    };
    Ok(FeedbackListPage {
        entries,
        next_cursor,
    })
}
pub(crate) fn rebuild(root: &Path, connection: &Connection) -> Result<()> {
    let base = root.join("feedback");
    files::ensure_no_symlinks(&base)?;
    if !base.exists() {
        return Ok(());
    }
    let mut stack = vec![base];
    while let Some(directory) = stack.pop() {
        files::ensure_no_symlinks(&directory)?;
        for entry in std::fs::read_dir(&directory)? {
            let path_value = entry?.path();
            let operation = || -> Result<()> {
                files::ensure_no_symlinks(&path_value)?;
                if !path_value.is_file() {
                    return Err(invalid("non-regular canonical feedback"));
                }
                let file = std::fs::File::open(&path_value)?;
                if !file.metadata()?.is_file() {
                    return Err(invalid("non-regular canonical feedback"));
                }
                let mut bytes = Vec::new();
                file.take(MAX_FEEDBACK_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > MAX_FEEDBACK_BYTES {
                    return Err(invalid("canonical feedback exceeds 64 KiB"));
                }
                let record: FeedbackRecord = serde_json::from_slice(&bytes)?;
                if path(root, &record) != path_value {
                    return Err(invalid("canonical feedback path mismatch"));
                }
                validate(connection, &record)?;
                project(connection, &record)
            };
            files::ensure_no_symlinks(&path_value)?;
            if path_value.is_dir() {
                stack.push(path_value);
                continue;
            }
            operation().map_err(|error| {
                invalid(&format!("canonical file {}: {error}", path_value.display()))
            })?;
        }
    }
    Ok(())
}
