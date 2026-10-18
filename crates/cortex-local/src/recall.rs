//! Context preparation with exact rendered references and best-effort audit receipts.
use crate::{files, *};
use serde::Serialize;
use std::time::Instant;

pub struct RecallRequest {
    pub repository: String,
    pub query: String,
    pub limit: usize,
    pub budget_bytes: usize,
    pub origin: AuditOrigin,
    pub attribution: AuditAttribution,
    pub fallback_recent: bool,
}

#[derive(Debug, Serialize)]
pub struct RecallResponse {
    pub repository: String,
    pub context: String,
    pub query_id: String,
    pub delivery_id: Option<String>,
    pub audit_persisted: bool,
    pub delivery_consumption: &'static str,
    #[serde(skip)]
    pub selected_count: usize,
}

struct Rendered {
    text: String,
    references: Vec<AuditReference>,
}

fn render(records: &[Record], budget: usize) -> Result<Rendered> {
    let mut out = String::from("Cortex local evidence (advisory; source authority unchanged):\n");
    let marker = "<!-- cortex: memory clipped; cortex_memory_read returns more -->\n";
    let mut references = Vec::new();
    for record in records {
        let citation = format!(
            "[{}@{} {}] {} | repository={} authority={:?} source={}\n",
            record.id,
            record.revision,
            record.content_sha256,
            record.title.replace(['\r', '\n'], " "),
            record.repository,
            record.authority,
            record.source.locator.replace(['\r', '\n'], " ")
        );
        if out.len() + citation.len() + marker.len() > budget {
            out.push_str(marker);
            break;
        }
        out.push_str(&citation);
        let mut reference = AuditReference::from_record(record, false)?;
        for line in record.body.split_inclusive('\n') {
            let extra = usize::from(!line.ends_with('\n'));
            if out.len() + line.len() + extra + marker.len() > budget {
                reference.body_clipped = true;
                references.push(reference);
                out.push_str(marker);
                return Ok(Rendered {
                    text: out,
                    references,
                });
            }
            out.push_str(line);
            if extra != 0 {
                out.push('\n');
            }
        }
        references.push(reference);
    }
    Ok(Rendered {
        text: out,
        references,
    })
}

fn select(store: &LocalStore, request: &RecallRequest) -> Result<Vec<Record>> {
    if !(1..=100).contains(&request.limit)
        || !(256..=6144).contains(&request.budget_bytes)
        || request.query.len() > 4096
    {
        return Err(LocalError::Invalid("invalid recall bounds".into()));
    }
    let mut records = if request.query.trim().is_empty() {
        store.recent_memories(&request.repository, request.limit, false)?
    } else {
        store
            .search_memories(SearchQuery {
                repository: request.repository.clone(),
                text: request.query.clone(),
                limit: request.limit,
                include_inactive: false,
            })?
            .into_iter()
            .map(|hit| hit.record)
            .collect()
    };
    if request.fallback_recent && records.len() < request.limit {
        for record in store.recent_memories(&request.repository, request.limit, false)? {
            if records.len() < request.limit && !records.iter().any(|old| old.id == record.id) {
                records.push(record);
            }
        }
    }
    Ok(records)
}

pub fn prepare(store: &LocalStore, request: RecallRequest) -> Result<RecallResponse> {
    let started = Instant::now();
    files::validate_scope(&request.repository)?;
    let query_id = format!("query-{}", ulid::Ulid::new());
    let prepared = select(store, &request).and_then(|records| {
        render(&records, request.budget_bytes).map(|rendered| (records.len(), rendered))
    });
    let (selected_count, context, references, outcome, error) = match &prepared {
        Ok((count, rendered)) => (
            *count,
            rendered.text.clone(),
            rendered.references.clone(),
            if rendered.references.is_empty() {
                PreparationOutcome::Empty
            } else {
                PreparationOutcome::Prepared
            },
            None,
        ),
        Err(_) => (
            0,
            String::new(),
            Vec::new(),
            PreparationOutcome::Error,
            Some(AuditErrorCode::PreparationFailed),
        ),
    };
    let receipt = store.append_audit(AuditRequest {
        id: None,
        repository: request.repository.clone(),
        payload: AuditPayload::Delivery(DeliveryDraft {
            query_id: query_id.clone(),
            origin: request.origin,
            attribution: request.attribution,
            query_sha256: files::hash(request.query.as_bytes()),
            context_sha256: files::hash(context.as_bytes()),
            context_bytes: context.len(),
            selected_count,
            omitted_count: selected_count - references.len(),
            references,
            outcome,
            latency_us: started.elapsed().as_micros().min(86_400_000_000) as u64,
            error,
        }),
    });
    prepared?;
    let delivery_id = receipt.ok().map(|written| written.record.id);
    Ok(RecallResponse {
        repository: request.repository,
        context,
        query_id,
        audit_persisted: delivery_id.is_some(),
        delivery_id,
        delivery_consumption: "not_observed",
        selected_count,
    })
}
