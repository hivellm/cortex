//! Durable evidence of prepared context, independent of model consumption.

use std::collections::BTreeSet;

use chrono::Utc;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{
    bounded, check_revision, invalid, read_record, revision, MemoryError, MemoryScope, MemoryStatus,
};
use crate::MetadataStore;

/// Exact record version included in the rendered context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryDeliveredItem {
    /// Effective repository or explicitly included global scope.
    pub scope: MemoryScope,
    /// Stable memory ID.
    pub id: String,
    /// Delivered version, rather than the version at later feedback time.
    pub version: u64,
    /// Original canonical capture event cited in the context.
    pub event_id: String,
}

/// Server-prepared delivery details; callers cannot claim observed consumption.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryDeliveryRequest {
    /// Exact repository used for this query, including queries with globals.
    pub repo: String,
    /// Resolved caller namespace, not an agent label used as authority.
    pub caller: String,
    /// Repository revision read with the query results.
    pub revision: u64,
    /// Global snapshot revision when globals were explicitly included.
    pub global_revision: Option<u64>,
    /// Only records actually rendered, at most five.
    pub memories: Vec<MemoryDeliveredItem>,
    /// Exact UTF-8 byte length of the rendered context, at most 1,536.
    pub context_bytes: usize,
    /// Preparation channel, such as http, mcp or pre_thinking; not delivery proof.
    pub channel: String,
}

/// The stage this store can prove without an acknowledgement from a harness.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDeliveryStage {
    /// Context was prepared and its receipt committed, not necessarily received.
    Prepared,
}

/// Persisted context receipt. Its ID is also the query ID used for feedback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryDelivery {
    /// Fresh query/delivery ID; not a fabricated source-session identity.
    pub delivery_id: String,
    /// Server preparation timestamp, RFC 3339 UTC.
    pub prepared_at: String,
    /// Evidence stage, always prepared for this operation.
    pub stage: MemoryDeliveryStage,
    /// Exact scoped snapshot and emitted record versions.
    pub request: MemoryDeliveryRequest,
}

impl MetadataStore {
    /// Commit a prepared receipt only if its rendered snapshot is still current.
    ///
    /// On revision conflict, the caller must rebuild context or fail explicitly;
    /// it must not return a receipt for superseded advice. This method never
    /// changes the corpus revision. Feedback uses the returned ID as query_id.
    pub fn record_daily_memory_delivery(
        &mut self,
        request: &MemoryDeliveryRequest,
    ) -> Result<MemoryDelivery, MemoryError> {
        let scope = MemoryScope::Repository(request.repo.clone()).key()?;
        bounded("caller", &request.caller, 256)?;
        bounded("channel", &request.channel, 64)?;
        if request.memories.len() > 5 || request.context_bytes > 1536 {
            return Err(invalid(
                "delivery exceeds five records or 1536 context bytes",
            ));
        }
        if request.memories.is_empty() != (request.context_bytes == 0) {
            return Err(invalid(
                "empty delivery must have empty context and conversely",
            ));
        }
        let tx = self
            .conn_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_revision(Some(request.revision), revision(&tx, &scope)?)?;
        if let Some(expected) = request.global_revision {
            check_revision(Some(expected), revision(&tx, "global")?)?;
        }
        let mut seen = BTreeSet::new();
        for item in &request.memories {
            let item_scope = item.scope.key()?;
            if item_scope != scope
                && !(item.scope == MemoryScope::Global && request.global_revision.is_some())
            {
                return Err(invalid(
                    "delivery contains a record outside the query scope",
                ));
            }
            bounded("memory id", &item.id, 256)?;
            if !seen.insert((item_scope.clone(), &item.id)) {
                return Err(invalid("delivery contains a duplicate record"));
            }
            let record = read_record(&tx, &item_scope, &item.id)?.ok_or(MemoryError::NotFound)?;
            if record.version != item.version
                || record.event_id != item.event_id
                || record.status != MemoryStatus::Accepted
                || record.stale
            {
                return Err(invalid(
                    "delivery record does not match eligible current version",
                ));
            }
        }
        let result = MemoryDelivery {
            delivery_id: cortex_core::event_id(),
            prepared_at: Utc::now().to_rfc3339(),
            stage: MemoryDeliveryStage::Prepared,
            request: request.clone(),
        };
        tx.execute(
            "INSERT INTO daily_memory_deliveries(id,repo,caller,prepared_at,data)
             VALUES(?1,?2,?3,?4,?5)",
            params![
                result.delivery_id,
                request.repo,
                request.caller,
                result.prepared_at,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }

    /// Look up a receipt within its effective repository, including after restart.
    pub fn daily_memory_delivery(
        &self,
        repo: &str,
        delivery_id: &str,
    ) -> Result<Option<MemoryDelivery>, MemoryError> {
        MemoryScope::Repository(repo.to_owned()).key()?;
        bounded("delivery id", delivery_id, 256)?;
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT data FROM daily_memory_deliveries WHERE repo=?1 AND id=?2",
                params![repo, delivery_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|data| Ok(serde_json::from_str(&data)?))
            .transpose()
    }

    /// Resolve a query ID for server-side feedback authorization.
    ///
    /// Unlike the scoped lookup, this returns the receipt's repository so the
    /// API can check access when legacy feedback carries only a query ID. The
    /// caller must authorize both repository and caller before exposing or
    /// modifying associated feedback. This method grants no access by itself.
    pub fn daily_memory_delivery_by_id(
        &self,
        delivery_id: &str,
    ) -> Result<Option<MemoryDelivery>, MemoryError> {
        bounded("delivery id", delivery_id, 256)?;
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT data FROM daily_memory_deliveries WHERE id=?1",
                params![delivery_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|data| Ok(serde_json::from_str(&data)?))
            .transpose()
    }
}
