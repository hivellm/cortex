//! Immutable, scoped evidence. Preparation receipts never imply model use.
use crate::{files, index, model::*};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_AUDIT_BYTES: usize = 64 * 1024;
macro_rules! audit_enum {
    ($name:ident { $($variant:ident),* }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),* }
    };
}
audit_enum!(AuditOrigin {
    Cli,
    Mcp,
    ClaudeSessionStart,
    ClaudeUserPromptSubmit,
    CodexSessionStart,
    CodexUserPromptSubmit,
    OpenCodeSessionStart,
    OpenCodeUserPromptSubmit
});
audit_enum!(PreparationOutcome {
    Prepared,
    Empty,
    Error
});
audit_enum!(LinkageMethod {
    NativeTurn,
    ExactCitation,
    ReportedUse,
    Unlinked
});
audit_enum!(EvidenceLabel {
    PreparedDelivery,
    ObservedRelatedAction,
    CapturedCitation,
    Unknown
});
audit_enum!(Verification {
    CanonicalEventCitation
});
audit_enum!(AuditErrorCode {
    PreparationFailed,
    StoreUnavailable,
    InvalidInput
});
audit_enum!(UncertaintyCode {
    MissingDelivery,
    AmbiguousDelivery,
    MissingAttribution,
    ReportedOnly
});
audit_enum!(AuditKind {
    Delivery,
    Observation,
    ArchiveControl
});

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditListQuery {
    pub repository: String,
    pub limit: usize,
    pub after: Option<String>,
    pub before: Option<String>,
    pub kind: Option<AuditKind>,
    pub cursor: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditListEntry {
    pub record: AuditRecord,
    pub archived: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditListPage {
    pub entries: Vec<AuditListEntry>,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditMetricsQuery {
    pub repository: String,
    pub after: Option<String>,
    pub before: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
    pub limit: usize,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditLatencyMetrics {
    pub samples: usize,
    pub min_us: Option<u64>,
    pub max_us: Option<u64>,
    pub sum_us: u64,
    pub mean_us: Option<f64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeliveryMetrics {
    pub attempts: usize,
    pub prepared: usize,
    pub empty: usize,
    pub errors: usize,
    pub unknown: usize,
    pub latency: AuditLatencyMetrics,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ObservationMetrics {
    pub total: usize,
    pub related: usize,
    pub captured_citations: usize,
    pub unknown: usize,
    pub reported_use: usize,
    pub missing_attribution: usize,
    pub missing_delivery: usize,
    pub ambiguous_delivery: usize,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditMetrics {
    pub matching_records: usize,
    pub sampled_records: usize,
    pub omitted_records: usize,
    pub truncated: bool,
    pub deliveries: DeliveryMetrics,
    pub observations: ObservationMetrics,
    pub archive_controls: usize,
    pub distinct_cited_deliveries: usize,
    pub prepared_deliveries_with_citation: usize,
    pub citation_delivery_rate: Option<f64>,
    pub citation_observations_with_delivery_outside_sample: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRetentionRequest {
    pub repository: String,
    pub cutoff: String,
    pub limit: usize,
    #[serde(default)]
    pub apply: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRetentionOutcome {
    pub delivery_ids: Vec<String>,
    pub linked_observations: usize,
    pub historical_references: usize,
    pub canonical_files_retained: bool,
    pub control: Option<AuditRecord>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveControlDraft {
    pub delivery_ids: Vec<String>,
    pub cutoff: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditAttribution {
    pub client: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub event_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditReference {
    pub repository: String,
    pub id: String,
    pub revision: u64,
    pub content_sha256: String,
    pub source_kind: String,
    pub source_sha256: String,
    pub body_clipped: bool,
}
impl AuditReference {
    pub fn from_record(record: &Record, body_clipped: bool) -> Result<Self> {
        Ok(Self {
            repository: record.repository.clone(),
            id: record.id.clone(),
            revision: record.revision,
            content_sha256: record.content_sha256.clone(),
            source_kind: record.source.kind.clone(),
            source_sha256: files::hash(&files::canonical(&record.source)?),
            body_clipped,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryDraft {
    pub query_id: String,
    pub origin: AuditOrigin,
    pub attribution: AuditAttribution,
    pub query_sha256: String,
    pub context_sha256: String,
    pub context_bytes: usize,
    pub selected_count: usize,
    pub omitted_count: usize,
    pub references: Vec<AuditReference>,
    pub outcome: PreparationOutcome,
    pub latency_us: u64,
    pub error: Option<AuditErrorCode>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationDraft {
    pub delivery_id: Option<String>,
    pub event: Option<AuditReference>,
    pub attribution: AuditAttribution,
    pub method: LinkageMethod,
    pub evidence: Vec<AuditReference>,
    pub uncertainty: Option<UncertaintyCode>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "content",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AuditPayload {
    Delivery(DeliveryDraft),
    Observation(ObservationDraft),
    ArchiveControl(ArchiveControlDraft),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRequest {
    pub id: Option<String>,
    pub repository: String,
    pub payload: AuditPayload,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditRecord {
    pub schema_version: u32,
    pub id: String,
    pub repository: String,
    pub recorded_at: String,
    pub payload: AuditPayload,
    pub evidence_label: EvidenceLabel,
    pub verification: Option<Verification>,
    pub content_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditWriteOutcome {
    pub record: AuditRecord,
    pub created: bool,
}

fn invalid(message: &str) -> LocalError {
    LocalError::Invalid(format!("audit: {message}"))
}
pub(crate) fn bounded_id(id: &str) -> Result<()> {
    files::validate_id(id)?;
    if id.len() > 256 {
        return Err(invalid("ID exceeds 256 bytes"));
    }
    Ok(())
}
fn hash_valid(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn timestamp(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>> {
    let time = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("invalid UTC timestamp"))?;
    if time.offset().local_minus_utc() != 0 {
        return Err(invalid("timestamp must be UTC"));
    }
    Ok(time)
}
fn sortable_timestamp(value: &str) -> Result<String> {
    // Fixed UTC precision preserves every accepted RFC3339 instant in SQLite order.
    Ok(timestamp(value)?.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
}
pub(crate) fn attribution(value: &AuditAttribution) -> Result<()> {
    for id in [
        &value.client,
        &value.session_id,
        &value.turn_id,
        &value.event_id,
        &value.agent_id,
    ]
    .into_iter()
    .flatten()
    {
        bounded_id(id)?;
    }
    Ok(())
}
fn reference(connection: &Connection, repository: &str, value: &AuditReference) -> Result<Record> {
    bounded_id(&value.id)?;
    if value.repository != repository
        || value.source_kind.len() > 128
        || !hash_valid(&value.content_sha256)
        || !hash_valid(&value.source_sha256)
        || value.revision == 0
        || value.revision > i64::MAX as u64
    {
        return Err(invalid("invalid or cross-scope reference"));
    }
    let record = index::get(connection, repository, &value.id, Some(value.revision))?
        .ok_or_else(|| invalid("missing historical revision"))?;
    if AuditReference::from_record(&record, value.body_clipped)? != *value {
        return Err(invalid("historical reference fingerprint mismatch"));
    }
    Ok(record)
}
fn references(connection: &Connection, repository: &str, values: &[AuditReference]) -> Result<()> {
    if values.len() > 100 {
        return Err(invalid("too many references"));
    }
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert((&value.repository, &value.id, value.revision)) {
            return Err(invalid("duplicate reference"));
        }
        reference(connection, repository, value)?;
    }
    Ok(())
}

pub(crate) fn derive(
    connection: &Connection,
    repository: &str,
    payload: &AuditPayload,
) -> Result<(EvidenceLabel, Option<Verification>)> {
    match payload {
        AuditPayload::ArchiveControl(control) => {
            let cutoff = timestamp(&control.cutoff)?;
            if control.delivery_ids.is_empty() || control.delivery_ids.len() > 100 {
                return Err(invalid("archive requires 1..100 delivery IDs"));
            }
            let mut seen = HashSet::new();
            for id in &control.delivery_ids {
                bounded_id(id)?;
                if !seen.insert(id) {
                    return Err(invalid("duplicate archive delivery"));
                }
                let target = get(connection, repository, id)?
                    .ok_or_else(|| invalid("missing scoped archive delivery"))?;
                if !matches!(target.payload, AuditPayload::Delivery(_))
                    || timestamp(&target.recorded_at)? >= cutoff
                {
                    return Err(invalid(
                        "archive target must be delivery strictly before cutoff",
                    ));
                }
            }
            Ok((EvidenceLabel::Unknown, None))
        }
        AuditPayload::Delivery(d) => {
            bounded_id(&d.query_id)?;
            attribution(&d.attribution)?;
            references(connection, repository, &d.references)?;
            if !hash_valid(&d.query_sha256)
                || !hash_valid(&d.context_sha256)
                || d.context_bytes > 6144
                || d.selected_count > 100
                || d.omitted_count > d.selected_count
                || d.references.len() != d.selected_count - d.omitted_count
                || d.latency_us > 86_400_000_000
            {
                return Err(invalid("invalid delivery bounds or counts"));
            }
            let valid = match d.outcome {
                PreparationOutcome::Prepared => {
                    !d.references.is_empty() && d.context_bytes > 0 && d.error.is_none()
                }
                PreparationOutcome::Empty => d.references.is_empty() && d.error.is_none(),
                PreparationOutcome::Error => {
                    d.references.is_empty() && d.context_bytes == 0 && d.error.is_some()
                }
            };
            if !valid {
                return Err(invalid("inconsistent preparation outcome"));
            }
            Ok((
                if d.outcome == PreparationOutcome::Prepared {
                    EvidenceLabel::PreparedDelivery
                } else {
                    EvidenceLabel::Unknown
                },
                None,
            ))
        }
        AuditPayload::Observation(o) => {
            attribution(&o.attribution)?;
            references(connection, repository, &o.evidence)?;
            let delivery = o
                .delivery_id
                .as_ref()
                .map(|id| {
                    bounded_id(id)?;
                    get(connection, repository, id)?
                        .ok_or_else(|| invalid("missing scoped delivery"))
                })
                .transpose()?;
            let event = o
                .event
                .as_ref()
                .map(|r| reference(connection, repository, r))
                .transpose()?;
            if let Some(event) = &event {
                if event.kind != RecordKind::Event
                    || event.source.kind != "lifecycle_hook"
                    || !matches!(
                        event.metadata["event"].as_str(),
                        Some("PostToolUse" | "Stop" | "SubagentStop")
                    )
                {
                    return Err(invalid("observation requires canonical lifecycle action"));
                }
            }
            let draft = match &delivery {
                Some(record) => match &record.payload {
                    AuditPayload::Delivery(d) => Some(d),
                    _ => return Err(invalid("link target is not delivery")),
                },
                None => None,
            };
            if let (Some(delivery), Some(event)) = (&delivery, &event) {
                if timestamp(&event.created_at)? < timestamp(&delivery.recorded_at)? {
                    return Err(invalid("action predates delivery"));
                }
            }
            for evidence in &o.evidence {
                if !draft.is_some_and(|d| d.references.contains(evidence)) {
                    return Err(invalid("evidence was not rendered"));
                }
            }
            if matches!(
                o.method,
                LinkageMethod::ReportedUse | LinkageMethod::Unlinked
            ) {
                let uncertainty_valid = match o.method {
                    LinkageMethod::ReportedUse => {
                        o.uncertainty == Some(UncertaintyCode::ReportedOnly)
                    }
                    LinkageMethod::Unlinked => matches!(
                        o.uncertainty,
                        Some(
                            UncertaintyCode::MissingDelivery
                                | UncertaintyCode::AmbiguousDelivery
                                | UncertaintyCode::MissingAttribution
                        )
                    ),
                    _ => false,
                };
                if !o.evidence.is_empty()
                    || (o.method == LinkageMethod::Unlinked && o.delivery_id.is_some())
                    || !uncertainty_valid
                {
                    return Err(invalid("inconsistent unverified observation"));
                }
                return Ok((EvidenceLabel::Unknown, None));
            }
            let d = draft.ok_or_else(|| invalid("link requires delivery"))?;
            if d.outcome != PreparationOutcome::Prepared {
                return Err(invalid("related action requires prepared delivery"));
            }
            let event = event.ok_or_else(|| invalid("link requires event"))?;
            for (key, supplied, delivered) in [
                ("client", &o.attribution.client, &d.attribution.client),
                (
                    "session_id",
                    &o.attribution.session_id,
                    &d.attribution.session_id,
                ),
                ("turn_id", &o.attribution.turn_id, &d.attribution.turn_id),
            ] {
                if supplied.is_none()
                    || supplied != delivered
                    || event.metadata[key].as_str() != supplied.as_deref()
                {
                    return Err(invalid("incomplete or mismatched native attribution"));
                }
            }
            let event_agent = event.metadata["agent_id"].as_str();
            if (!event.metadata["agent_id"].is_null()
                && event_agent.map_or(true, |value| bounded_id(value).is_err()))
                || o.attribution.agent_id.as_deref() != event_agent
                || d.attribution.agent_id.as_deref() != event_agent
            {
                return Err(invalid("missing or mismatched agent attribution"));
            }
            if let Some(event_id) = &o.attribution.event_id {
                if event.metadata["event_id"].as_str() != Some(event_id.as_str()) {
                    return Err(invalid("event attribution differs from canonical action"));
                }
            }
            if o.uncertainty.is_some() {
                return Err(invalid("verified linkage cannot carry uncertainty"));
            }
            if o.method == LinkageMethod::NativeTurn {
                if !o.evidence.is_empty() {
                    return Err(invalid(
                        "native-turn correlation cannot claim citation evidence",
                    ));
                }
                return Ok((EvidenceLabel::ObservedRelatedAction, None));
            }
            if o.evidence.is_empty() || event.metadata["event"] != "Stop" {
                return Err(invalid("exact citation requires nonempty Stop evidence"));
            }
            for evidence in &o.evidence {
                let citation = format!(
                    "[{}@{} {}]",
                    evidence.id, evidence.revision, evidence.content_sha256
                );
                if !event.body.contains(&citation) {
                    return Err(invalid("canonical assistant body lacks exact citation"));
                }
            }
            Ok((
                EvidenceLabel::CapturedCitation,
                Some(Verification::CanonicalEventCitation),
            ))
        }
    }
}

pub(crate) fn make(connection: &Connection, request: AuditRequest) -> Result<AuditRecord> {
    files::validate_scope(&request.repository)?;
    let id = request
        .id
        .unwrap_or_else(|| format!("audit-{}", ulid::Ulid::new()));
    bounded_id(&id)?;
    let (evidence_label, verification) = derive(connection, &request.repository, &request.payload)?;
    let mut record = AuditRecord {
        schema_version: 1,
        id,
        repository: request.repository,
        recorded_at: chrono::Utc::now().to_rfc3339(),
        payload: request.payload,
        evidence_label,
        verification,
        content_sha256: String::new(),
    };
    record.content_sha256 = files::hash(&files::canonical(&record)?);
    validate(connection, &record)?;
    Ok(record)
}

pub(crate) fn observation_identity(
    connection: &Connection,
    event: &Record,
) -> Result<(String, AuditReference)> {
    let event_reference = AuditReference::from_record(event, false)?;
    let canonical = reference(connection, &event.repository, &event_reference)?;
    if canonical != *event
        || event.kind != RecordKind::Event
        || event.source.kind != "lifecycle_hook"
        || !matches!(
            event.metadata["event"].as_str(),
            Some("PostToolUse" | "Stop" | "SubagentStop")
        )
    {
        return Err(invalid(
            "observation requires exact canonical lifecycle action",
        ));
    }
    timestamp(&event.created_at)?;
    let identity = files::canonical(&(
        event.repository.as_str(),
        event.id.as_str(),
        event.revision,
        event.content_sha256.as_str(),
        event_reference.source_sha256.as_str(),
    ))?;
    Ok((
        format!("observation-{}", files::hash(&identity)),
        event_reference,
    ))
}

pub(crate) fn observation_request(
    connection: &Connection,
    event: &Record,
    id: String,
    event_reference: AuditReference,
) -> Result<AuditRequest> {
    fn observed_id(event: &Record, key: &str) -> Option<String> {
        event.metadata[key]
            .as_str()
            .filter(|value| bounded_id(value).is_ok())
            .map(str::to_owned)
    }
    let attribution = AuditAttribution {
        client: observed_id(event, "client"),
        session_id: observed_id(event, "session_id"),
        turn_id: observed_id(event, "turn_id"),
        event_id: observed_id(event, "event_id"),
        agent_id: observed_id(event, "agent_id"),
    };
    let mut observation = ObservationDraft {
        delivery_id: None,
        event: Some(event_reference),
        attribution,
        method: LinkageMethod::Unlinked,
        evidence: Vec::new(),
        uncertainty: Some(UncertaintyCode::MissingAttribution),
    };
    let agent_valid =
        event.metadata["agent_id"].is_null() || observation.attribution.agent_id.is_some();
    if let (Some(client), Some(session), Some(turn), true) = (
        &observation.attribution.client,
        &observation.attribution.session_id,
        &observation.attribution.turn_id,
        agent_valid,
    ) {
        let mut statement = connection.prepare(
            "SELECT record FROM audit_records WHERE repository=?1 AND recorded_at<=?2
            AND json_extract(record,'$.payload.type')='delivery'
            AND json_extract(record,'$.payload.content.outcome')='prepared'
            AND json_extract(record,'$.payload.content.attribution.client')=?3
            AND json_extract(record,'$.payload.content.attribution.session_id')=?4
            AND json_extract(record,'$.payload.content.attribution.turn_id')=?5
            AND json_extract(record,'$.payload.content.attribution.agent_id') IS ?6
            ORDER BY recorded_at DESC,id DESC LIMIT 2",
        )?;
        let candidates = statement
            .query_map(
                params![
                    event.repository,
                    sortable_timestamp(&event.created_at)?,
                    client,
                    session,
                    turn,
                    observation.attribution.agent_id
                ],
                |row| row.get::<_, String>(0),
            )?
            .map(|row| Ok(serde_json::from_str::<AuditRecord>(&row?)?))
            .collect::<Result<Vec<_>>>()?;
        match candidates.as_slice() {
            [] => observation.uncertainty = Some(UncertaintyCode::MissingDelivery),
            [candidate] => {
                observation.delivery_id = Some(candidate.id.clone());
                observation.method = LinkageMethod::NativeTurn;
                observation.uncertainty = None;
                if event.metadata["event"] == "Stop" {
                    if let AuditPayload::Delivery(delivery) = &candidate.payload {
                        observation.evidence = delivery
                            .references
                            .iter()
                            .filter(|reference| {
                                event.body.contains(&format!(
                                    "[{}@{} {}]",
                                    reference.id, reference.revision, reference.content_sha256
                                ))
                            })
                            .cloned()
                            .collect();
                        if !observation.evidence.is_empty() {
                            observation.method = LinkageMethod::ExactCitation;
                        }
                    }
                }
            }
            _ => observation.uncertainty = Some(UncertaintyCode::AmbiguousDelivery),
        }
    }
    Ok(AuditRequest {
        id: Some(id),
        repository: event.repository.clone(),
        payload: AuditPayload::Observation(observation),
    })
}
pub(crate) fn validate(connection: &Connection, record: &AuditRecord) -> Result<()> {
    files::validate_scope(&record.repository)?;
    bounded_id(&record.id)?;
    timestamp(&record.recorded_at)?;
    if record.schema_version != 1 || files::canonical(record)?.len() > MAX_AUDIT_BYTES {
        return Err(invalid("schema or size bound"));
    }
    let mut unsigned = record.clone();
    unsigned.content_sha256.clear();
    if files::hash(&files::canonical(&unsigned)?) != record.content_sha256 {
        return Err(invalid("content hash mismatch"));
    }
    if derive(connection, &record.repository, &record.payload)?
        != (record.evidence_label, record.verification)
    {
        return Err(invalid("evidence label mismatch"));
    }
    Ok(())
}
pub(crate) fn path(root: &Path, record: &AuditRecord) -> PathBuf {
    root.join("audit")
        .join(files::hash(record.repository.as_bytes()))
        .join(format!("{}.json", files::hash(record.id.as_bytes())))
}
pub(crate) fn get(
    connection: &Connection,
    repository: &str,
    id: &str,
) -> Result<Option<AuditRecord>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT record FROM audit_records WHERE repository=?1 AND id=?2",
            params![repository, id],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|s| serde_json::from_str(&s).map_err(LocalError::from))
        .transpose()
}
pub(crate) fn project(connection: &Connection, record: &AuditRecord) -> Result<()> {
    connection.execute(
        "INSERT INTO audit_records(repository,id,recorded_at,record) VALUES(?1,?2,?3,?4)",
        params![
            record.repository,
            record.id,
            sortable_timestamp(&record.recorded_at)?,
            serde_json::to_string(record)?
        ],
    )?;
    if let AuditPayload::ArchiveControl(control) = &record.payload {
        for id in &control.delivery_ids {
            connection.execute(
                "INSERT OR IGNORE INTO audit_archived(repository,delivery_id) VALUES(?1,?2)",
                params![record.repository, id],
            )?;
        }
    }
    Ok(())
}

fn kind_name(kind: AuditKind) -> &'static str {
    match kind {
        AuditKind::Delivery => "delivery",
        AuditKind::Observation => "observation",
        AuditKind::ArchiveControl => "archive_control",
    }
}

pub(crate) fn list(connection: &Connection, query: &AuditListQuery) -> Result<AuditListPage> {
    files::validate_scope(&query.repository)?;
    if !(1..=100).contains(&query.limit) {
        return Err(invalid("list limit must be 1..100"));
    }
    let after = query.after.as_deref().map(sortable_timestamp).transpose()?;
    let before = query
        .before
        .as_deref()
        .map(sortable_timestamp)
        .transpose()?;
    if after
        .as_ref()
        .zip(before.as_ref())
        .is_some_and(|(a, b)| a >= b)
    {
        return Err(invalid("after must precede before"));
    }
    let cursor_time = if let Some(id) = &query.cursor {
        bounded_id(id)?;
        let record = get(connection, &query.repository, id)?
            .ok_or_else(|| invalid("cursor not found in exact scope"))?;
        Some(sortable_timestamp(&record.recorded_at)?)
    } else {
        None
    };
    let mut statement = connection.prepare(
        "SELECT a.record, EXISTS(SELECT 1 FROM audit_archived x WHERE x.repository=a.repository AND x.delivery_id=CASE json_extract(a.record,'$.payload.type') WHEN 'delivery' THEN a.id WHEN 'observation' THEN json_extract(a.record,'$.payload.content.delivery_id') END) AS archived
         FROM audit_records a WHERE a.repository=?1
         AND (?2 IS NULL OR a.recorded_at>=?2) AND (?3 IS NULL OR a.recorded_at<?3)
         AND (?4 IS NULL OR json_extract(a.record,'$.payload.type')=?4)
         AND (?5 IS NULL OR a.recorded_at<?5 OR (a.recorded_at=?5 AND a.id<?6))
         AND (?7 OR NOT EXISTS(SELECT 1 FROM audit_archived x WHERE x.repository=a.repository AND x.delivery_id=CASE json_extract(a.record,'$.payload.type') WHEN 'delivery' THEN a.id WHEN 'observation' THEN json_extract(a.record,'$.payload.content.delivery_id') END))
         ORDER BY a.recorded_at DESC,a.id DESC LIMIT ?8")?;
    let rows = statement.query_map(
        params![
            query.repository,
            after,
            before,
            query.kind.map(kind_name),
            cursor_time,
            query.cursor,
            query.include_archived,
            (query.limit + 1) as i64
        ],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
    )?;
    let mut entries = Vec::new();
    for row in rows {
        let (json, archived) = row?;
        entries.push(AuditListEntry {
            record: serde_json::from_str(&json)?,
            archived,
        });
    }
    let next_cursor = if entries.len() > query.limit {
        entries.truncate(query.limit);
        entries.last().map(|e| e.record.id.clone())
    } else {
        None
    };
    Ok(AuditListPage {
        entries,
        next_cursor,
    })
}

pub(crate) fn retention_plan(
    connection: &Connection,
    request: &AuditRetentionRequest,
) -> Result<AuditRetentionOutcome> {
    timestamp(&request.cutoff)?;
    let page = list(
        connection,
        &AuditListQuery {
            repository: request.repository.clone(),
            limit: request.limit,
            after: None,
            before: Some(request.cutoff.clone()),
            kind: Some(AuditKind::Delivery),
            cursor: None,
            include_archived: false,
        },
    )?;
    let mut outcome = AuditRetentionOutcome {
        delivery_ids: Vec::new(),
        linked_observations: 0,
        historical_references: 0,
        canonical_files_retained: true,
        control: None,
    };
    let mut seen = HashSet::new();
    for entry in page.entries {
        let AuditPayload::Delivery(delivery) = entry.record.payload else {
            return Err(invalid("retention selection is not delivery"));
        };
        let count: i64 = connection.query_row("SELECT count(*) FROM audit_records WHERE repository=?1 AND json_extract(record,'$.payload.type')='observation' AND json_extract(record,'$.payload.content.delivery_id')=?2", params![request.repository, entry.record.id], |row| row.get(0))?;
        outcome.linked_observations +=
            usize::try_from(count).map_err(|_| invalid("observation count overflow"))?;
        outcome.delivery_ids.push(entry.record.id);
        for reference in delivery.references {
            if seen.insert((
                reference.repository,
                reference.id,
                reference.revision,
                reference.content_sha256,
                reference.source_sha256,
            )) {
                outcome.historical_references += 1;
            }
        }
    }
    Ok(outcome)
}

pub(crate) fn metrics(connection: &Connection, query: &AuditMetricsQuery) -> Result<AuditMetrics> {
    files::validate_scope(&query.repository)?;
    if !(1..=10_000).contains(&query.limit) {
        return Err(invalid("metrics limit must be 1..10000"));
    }
    let after = query.after.as_deref().map(sortable_timestamp).transpose()?;
    let before = query
        .before
        .as_deref()
        .map(sortable_timestamp)
        .transpose()?;
    if after
        .as_ref()
        .zip(before.as_ref())
        .is_some_and(|(a, b)| a >= b)
    {
        return Err(invalid("after must precede before"));
    }
    let filter = "FROM audit_records a WHERE a.repository=?1
        AND (?2 IS NULL OR a.recorded_at>=?2) AND (?3 IS NULL OR a.recorded_at<?3)
        AND (?4 OR NOT EXISTS(SELECT 1 FROM audit_archived x WHERE x.repository=a.repository AND x.delivery_id=CASE json_extract(a.record,'$.payload.type') WHEN 'delivery' THEN a.id WHEN 'observation' THEN json_extract(a.record,'$.payload.content.delivery_id') END))";
    let count: i64 = connection.query_row(
        &format!("SELECT count(*) {filter}"),
        params![query.repository, after, before, query.include_archived],
        |row| row.get(0),
    )?;
    let matching_records = usize::try_from(count).map_err(|_| invalid("metrics count overflow"))?;
    let mut output = AuditMetrics {
        matching_records,
        ..Default::default()
    };
    let mut prepared_ids = HashSet::new();
    let mut cited_ids: HashMap<String, usize> = HashMap::new();
    let mut statement = connection.prepare(&format!(
        "SELECT a.record {filter} ORDER BY a.recorded_at DESC,a.id DESC LIMIT ?5"
    ))?;
    let mut rows = statement.query(params![
        query.repository,
        after,
        before,
        query.include_archived,
        query.limit as i64
    ])?;
    // Keep only one canonical payload in memory; cross-row aggregation stores bounded IDs.
    while let Some(row) = rows.next()? {
        let record: AuditRecord = serde_json::from_str(&row.get::<_, String>(0)?)?;
        output.sampled_records += 1;
        match record.payload {
            AuditPayload::Delivery(delivery) => {
                let counters = &mut output.deliveries;
                counters.attempts += 1;
                match delivery.outcome {
                    PreparationOutcome::Prepared => {
                        counters.prepared += 1;
                        prepared_ids.insert(record.id);
                    }
                    PreparationOutcome::Empty => {
                        counters.empty += 1;
                        counters.unknown += 1;
                    }
                    PreparationOutcome::Error => {
                        counters.errors += 1;
                        counters.unknown += 1;
                    }
                }
                let latency = &mut counters.latency;
                latency.samples += 1;
                latency.min_us = Some(
                    latency
                        .min_us
                        .map_or(delivery.latency_us, |old| old.min(delivery.latency_us)),
                );
                latency.max_us = Some(
                    latency
                        .max_us
                        .map_or(delivery.latency_us, |old| old.max(delivery.latency_us)),
                );
                latency.sum_us = latency
                    .sum_us
                    .checked_add(delivery.latency_us)
                    .ok_or_else(|| invalid("latency sum overflow"))?;
            }
            AuditPayload::Observation(observation) => {
                let counters = &mut output.observations;
                counters.total += 1;
                match record.evidence_label {
                    EvidenceLabel::ObservedRelatedAction => counters.related += 1,
                    EvidenceLabel::CapturedCitation => {
                        counters.captured_citations += 1;
                        let id = observation
                            .delivery_id
                            .as_ref()
                            .ok_or_else(|| invalid("cited observation missing delivery"))?;
                        *cited_ids.entry(id.clone()).or_default() += 1;
                    }
                    EvidenceLabel::Unknown => counters.unknown += 1,
                    EvidenceLabel::PreparedDelivery => {
                        return Err(invalid("observation carries delivery label"))
                    }
                }
                if observation.method == LinkageMethod::ReportedUse {
                    counters.reported_use += 1;
                }
                match observation.uncertainty {
                    Some(UncertaintyCode::MissingAttribution) => counters.missing_attribution += 1,
                    Some(UncertaintyCode::MissingDelivery) => counters.missing_delivery += 1,
                    Some(UncertaintyCode::AmbiguousDelivery) => counters.ambiguous_delivery += 1,
                    _ => {}
                }
            }
            AuditPayload::ArchiveControl(_) => output.archive_controls += 1,
        }
    }
    output.omitted_records = output
        .matching_records
        .checked_sub(output.sampled_records)
        .ok_or_else(|| invalid("metrics population changed during snapshot"))?;
    output.truncated = output.omitted_records > 0;
    let latency = &mut output.deliveries.latency;
    if latency.samples > 0 {
        latency.mean_us = Some(latency.sum_us as f64 / latency.samples as f64);
    }
    output.distinct_cited_deliveries = cited_ids.len();
    output.prepared_deliveries_with_citation = cited_ids
        .keys()
        .filter(|id| prepared_ids.contains(*id))
        .count();
    output.citation_observations_with_delivery_outside_sample = cited_ids
        .iter()
        .filter(|(id, _)| !prepared_ids.contains(*id))
        .map(|(_, count)| count)
        .sum();
    if output.deliveries.prepared > 0 {
        output.citation_delivery_rate = Some(
            output.prepared_deliveries_with_citation as f64 / output.deliveries.prepared as f64,
        );
    }
    Ok(output)
}
pub(crate) fn rebuild(root: &Path, connection: &Connection) -> Result<()> {
    let base = root.join("audit");
    files::ensure_no_symlinks(&base).map_err(|error| file_error(&base, error))?;
    if !base.exists() {
        return Ok(());
    }
    let mut pending = vec![base];
    while let Some(directory) = pending.pop() {
        files::ensure_no_symlinks(&directory).map_err(|error| file_error(&directory, error))?;
        for entry in std::fs::read_dir(&directory).map_err(|error| file_error(&directory, error))? {
            let path_value = entry.map_err(|error| file_error(&directory, error))?.path();
            files::ensure_no_symlinks(&path_value)
                .map_err(|error| file_error(&path_value, error))?;
            if path_value.is_dir() {
                pending.push(path_value);
                continue;
            }
            read_and_project(root, connection, &path_value)
                .map_err(|error| file_error(&path_value, error))?;
        }
    }
    // Load all receipts first: filesystem order cannot determine linkage validity.
    let mut statement = connection.prepare("SELECT record FROM audit_records")?;
    for json in statement.query_map([], |row| row.get::<_, String>(0))? {
        let record: AuditRecord = serde_json::from_str(&json?)?;
        validate(connection, &record).map_err(|error| file_error(&path(root, &record), error))?;
    }
    Ok(())
}

fn file_error(path: &Path, error: impl std::fmt::Display) -> LocalError {
    invalid(&format!("canonical file {}: {error}", path.display()))
}

fn read_and_project(root: &Path, connection: &Connection, path_value: &Path) -> Result<()> {
    if !path_value.is_file() {
        return Err(invalid("invalid canonical file"));
    }
    let file = std::fs::File::open(path_value)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_AUDIT_BYTES as u64 {
        return Err(invalid("invalid canonical file"));
    }
    // The bound applies to the read itself even if a noncooperating writer grows the file.
    let mut bytes = Vec::new();
    file.take(MAX_AUDIT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_AUDIT_BYTES {
        return Err(invalid("canonical file exceeds 64 KiB"));
    }
    let record: AuditRecord = serde_json::from_slice(&bytes)?;
    if path(root, &record) != path_value {
        return Err(invalid("canonical path mismatch"));
    }
    project(connection, &record)
}
