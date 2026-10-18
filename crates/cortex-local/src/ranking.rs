//! Opt-in bounded feedback adjustment within unchanged literal-search candidates.
use crate::{files, index, *};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const FEEDBACK_POLICY: &str = "local-feedback-v1";
pub const FEEDBACK_CANDIDATE_LIMIT: usize = 100;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankingOptions {
    #[serde(default)]
    pub enabled: bool,
    pub as_of: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackRankingReason {
    Applied,
    InsufficientSamples,
    InsufficientWeight,
    NoEligibleEvidence,
    EvidenceLimit,
    EventExcluded,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackExplanation {
    pub policy: String,
    pub as_of: String,
    pub baseline_score: f64,
    pub adjustment: f64,
    pub final_score: f64,
    pub samples: usize,
    pub conflicting_groups: usize,
    pub effective_weight: f64,
    pub reason: FeedbackRankingReason,
    pub candidate_limit: usize,
    pub evidence_records: usize,
    pub deliveries: usize,
}
fn invalid(message: &str) -> LocalError {
    LocalError::Invalid(format!("feedback ranking: {message}"))
}
fn timestamp(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| invalid("invalid UTC as_of or evidence timestamp"))?;
    if parsed.offset().local_minus_utc() != 0 {
        return Err(invalid("timestamp must be UTC"));
    }
    Ok(parsed)
}
struct Delivery {
    query_id: String,
    query_hash: String,
    prepared: bool,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct RecordKey {
    id: String,
    revision: u64,
    content_hash: String,
    source_hash: String,
}
impl RecordKey {
    fn reference(reference: &AuditReference) -> Self {
        Self {
            id: reference.id.clone(),
            revision: reference.revision,
            content_hash: reference.content_sha256.clone(),
            source_hash: reference.source_sha256.clone(),
        }
    }
    fn record(record: &Record) -> Result<Self> {
        Ok(Self {
            id: record.id.clone(),
            revision: record.revision,
            content_hash: record.content_sha256.clone(),
            source_hash: files::hash(&files::canonical(&record.source)?),
        })
    }
}
struct Group {
    helpful: bool,
    unhelpful: bool,
    earliest: chrono::DateTime<chrono::FixedOffset>,
}

pub(crate) fn search(
    connection: &Connection,
    mut query: SearchQuery,
    options: RankingOptions,
) -> Result<Vec<SearchHit>> {
    if !(1..=100).contains(&query.limit) {
        return Err(invalid("search limit must be 1..100"));
    }
    let as_of = match options.as_of.as_deref() {
        Some(value) => timestamp(value)?,
        None => chrono::Utc::now().fixed_offset(),
    };
    let as_of_text = as_of.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    let requested_limit = query.limit;
    query.limit = FEEDBACK_CANDIDATE_LIMIT;
    let repository = query.repository.clone();
    let query_hash = files::hash(query.text.as_bytes());
    let mut hits = index::search(connection, query, false)?;
    let evidence_records: i64 = connection.query_row(
        "SELECT count(*) FROM feedback_records WHERE repository=?1 AND recorded_at<=?2",
        params![repository, as_of_text],
        |r| r.get(0),
    )?;
    let delivery_count:i64=connection.query_row("SELECT count(*) FROM audit_records WHERE repository=?1 AND recorded_at<=?2 AND json_extract(record,'$.payload.type')='delivery'",params![repository,as_of_text],|r|r.get(0))?;
    let evidence_records =
        usize::try_from(evidence_records).map_err(|_| invalid("evidence count overflow"))?;
    let delivery_count =
        usize::try_from(delivery_count).map_err(|_| invalid("delivery count overflow"))?;
    let limited =
        evidence_records > MAX_FEEDBACK_EVIDENCE || delivery_count > MAX_FEEDBACK_EVIDENCE;
    let mut groups: BTreeMap<(RecordKey, String), Group> = BTreeMap::new();
    if !limited {
        let mut deliveries: HashMap<String, Delivery> = HashMap::new();
        let mut query_hashes: HashMap<String, String> = HashMap::new();
        let mut inconsistent = HashSet::new();
        let mut statement=connection.prepare("SELECT record FROM audit_records WHERE repository=?1 AND recorded_at<=?2 AND json_extract(record,'$.payload.type')='delivery' ORDER BY recorded_at,id")?;
        for row in
            statement.query_map(params![repository, as_of_text], |r| r.get::<_, String>(0))?
        {
            let record: AuditRecord = serde_json::from_str(&row?)?;
            let AuditPayload::Delivery(draft) = record.payload else {
                return Err(invalid("delivery projection mismatch"));
            };
            if query_hashes
                .get(&draft.query_id)
                .is_some_and(|old| old != &draft.query_sha256)
            {
                inconsistent.insert(draft.query_id.clone());
            }
            query_hashes.insert(draft.query_id.clone(), draft.query_sha256.clone());
            deliveries.insert(
                record.id,
                Delivery {
                    query_id: draft.query_id,
                    query_hash: draft.query_sha256,
                    prepared: draft.outcome == PreparationOutcome::Prepared,
                },
            );
        }
        let candidate_keys = hits
            .iter()
            .filter(|hit| hit.record.kind != RecordKind::Event)
            .map(|hit| RecordKey::record(&hit.record))
            .collect::<Result<HashSet<_>>>()?;
        let mut statement=connection.prepare("SELECT record FROM feedback_records WHERE repository=?1 AND recorded_at<=?2 ORDER BY recorded_at,id")?;
        for row in
            statement.query_map(params![repository, as_of_text], |r| r.get::<_, String>(0))?
        {
            let record: FeedbackRecord = serde_json::from_str(&row?)?;
            let request = record.request;
            if request.evidence != FeedbackEvidence::Explicit
                || request.judgment == FeedbackJudgment::Unknown
                || request.repository != repository
            {
                continue;
            }
            let (Some(reference), Some(query_id), Some(delivery_id)) =
                (request.reference, request.query_id, request.delivery_id)
            else {
                continue;
            };
            if reference.repository != repository || inconsistent.contains(&query_id) {
                continue;
            }
            let Some(delivery) = deliveries.get(&delivery_id) else {
                continue;
            };
            if !delivery.prepared
                || delivery.query_id != query_id
                || delivery.query_hash != query_hash
            {
                continue;
            }
            let key = RecordKey::reference(&reference);
            if !candidate_keys.contains(&key) {
                continue;
            }
            let recorded_at = timestamp(&record.recorded_at)?;
            if recorded_at > as_of {
                continue;
            }
            let group = groups.entry((key, query_id)).or_insert(Group {
                helpful: false,
                unhelpful: false,
                earliest: recorded_at,
            });
            group.earliest = group.earliest.min(recorded_at);
            match request.judgment {
                FeedbackJudgment::Helpful => group.helpful = true,
                FeedbackJudgment::Unhelpful => group.unhelpful = true,
                FeedbackJudgment::Unknown => {}
            }
        }
    }
    for hit in &mut hits {
        let mut explanation = FeedbackExplanation {
            policy: FEEDBACK_POLICY.into(),
            as_of: as_of_text.clone(),
            baseline_score: hit.score,
            adjustment: 0.0,
            final_score: hit.score,
            samples: 0,
            conflicting_groups: 0,
            effective_weight: 0.0,
            reason: FeedbackRankingReason::NoEligibleEvidence,
            candidate_limit: FEEDBACK_CANDIDATE_LIMIT,
            evidence_records,
            deliveries: delivery_count,
        };
        if limited {
            explanation.reason = FeedbackRankingReason::EvidenceLimit;
        } else if hit.record.kind == RecordKind::Event {
            explanation.reason = FeedbackRankingReason::EventExcluded;
        } else {
            let key = RecordKey::record(&hit.record)?;
            let mut signed_weight = 0.0;
            for ((record_key, _), group) in &groups {
                if record_key != &key {
                    continue;
                }
                if group.helpful && group.unhelpful {
                    explanation.conflicting_groups += 1;
                    continue;
                }
                let age_ms = (as_of - group.earliest).num_milliseconds().max(0) as f64;
                let weight = 2.0_f64.powf(-age_ms / 2_592_000_000.0);
                explanation.samples += 1;
                explanation.effective_weight += weight;
                signed_weight += if group.helpful { weight } else { -weight };
            }
            explanation.reason = if explanation.samples == 0 {
                FeedbackRankingReason::NoEligibleEvidence
            } else if explanation.samples < 3 {
                FeedbackRankingReason::InsufficientSamples
            } else if explanation.effective_weight < 1.5 {
                FeedbackRankingReason::InsufficientWeight
            } else {
                FeedbackRankingReason::Applied
            };
            if explanation.reason == FeedbackRankingReason::Applied {
                explanation.adjustment =
                    (0.25 * signed_weight / explanation.samples as f64).clamp(-0.25, 0.25);
                explanation.final_score = hit.score + explanation.adjustment;
                if !explanation.final_score.is_finite() || !explanation.effective_weight.is_finite()
                {
                    return Err(invalid("nonfinite ranking arithmetic"));
                }
                hit.score = explanation.final_score;
            }
        }
        hit.feedback = Some(explanation);
    }
    // Stable sorting retains the original literal-search order on equal adjusted scores.
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(requested_limit);
    Ok(hits)
}
