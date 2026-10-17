//! Durable admission and nonsecret accounting for optional candidate advice.
//!
//! Reservations precede network work. A crash retains its charge and unknown
//! usage; its short lease expires without resetting the UTC daily allowance.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{
    bounded, invalid, read_record, revision, MemoryError, MemoryReceipt, MemoryScope, MemoryStatus,
};
use crate::MetadataStore;

const DAILY_LIMIT: u64 = 100;
const LEASE_MS: i64 = 3000;
const CACHE_LIMIT: usize = 512;
const CACHE_BYTES: usize = 16 * 1024;

/// Persisted permission for one bounded provider attempt, not source identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdviceReservation {
    /// Generated accounting identity; finish this attempt at most once.
    pub attempt_id: String,
}

/// Atomic admission result. Cache hits spend no provider allowance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdviceAdmission {
    /// Previously validated successful JSON from the same key/model/schema.
    Cached(String),
    /// Durable charge and short global lease committed before network access.
    Reserved(AdviceReservation),
    /// All 100 uncached UTC-day reservations have been consumed.
    BudgetExhausted,
    /// Another attempt still holds the global three-second lease.
    Busy,
}

/// Actual reported usage, never inferred from text or per-call cent rounding.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdviceUsage {
    /// Input tokens reported by the provider.
    pub input_tokens: u64,
    /// Output tokens reported by the provider.
    pub output_tokens: u64,
}

/// Exact candidate/corpus snapshot observed before optional network work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdviceSnapshot {
    /// Authorized scope, never broadened by classifier output.
    pub scope: MemoryScope,
    /// Canonical capture receipt observed before classification.
    pub receipt: MemoryReceipt,
}

/// Nonsecret aggregate measurements for an explicit UTC calendar day.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdviceStats {
    /// Reservations charged on this day, including failures and crashes.
    pub reserved: u64,
    /// Attempts with an observed final outcome.
    pub finished: u64,
    /// Finished successful judgments, independent of their probabilities.
    pub succeeded: u64,
    /// Attempts with actual provider-reported usage.
    pub usage_known: u64,
    /// Reservations without usage, including unfinished/crashed attempts.
    pub usage_unknown: u64,
    /// Reported input tokens, summed with overflow protection.
    pub input_tokens: u64,
    /// Reported output tokens, summed with overflow protection.
    pub output_tokens: u64,
    /// Observed finished-attempt latency; unknown latency is not fabricated.
    pub latency_ms: u64,
    /// Successful cache lookups that made no reservation.
    pub cache_hits: u64,
    /// Calls denied by the persisted daily allowance.
    pub budget_exhausted: u64,
    /// Calls denied by the global lease.
    pub busy: u64,
    /// Bounded reason counts, without state, secrets or provider error bodies.
    pub fallbacks: BTreeMap<String, u64>,
}

impl MetadataStore {
    /// Check cache, global lease and daily allowance in one immediate transaction.
    pub fn reserve_daily_memory_advice(
        &mut self,
        cache_key: &str,
        model: &str,
        schema_version: &str,
    ) -> Result<AdviceAdmission, MemoryError> {
        reserve_at(self, cache_key, model, schema_version, Utc::now())
    }

    /// Finish once; failed attempts retain their charge and cannot populate cache.
    ///
    /// Cache JSON must already have passed semantic provider validation. This
    /// boundary additionally enforces object shape and the 16-KiB byte ceiling.
    /// A repeated finish keeps the original outcome, usage and cache unchanged.
    pub fn finish_daily_memory_advice(
        &mut self,
        attempt_id: &str,
        outcome: &str,
        usage: Option<AdviceUsage>,
        latency_ms: u64,
        cache_json: Option<&str>,
    ) -> Result<(), MemoryError> {
        finish(
            self,
            Completion {
                attempt_id,
                outcome,
                usage,
                latency_ms,
                cache_json,
            },
            None,
        )
        .map(|_| ())
    }

    /// Atomically reject obsolete candidate results before publishing any cache.
    ///
    /// Accepted and proposed candidates can receive advice; classification never
    /// promotes either. A changed corpus, record or source stores `stale` with
    /// actual usage/latency, skips cache, and returns false. Idempotent replay
    /// never changes the original accounting and still checks current validity.
    pub fn finish_daily_memory_advice_if_current(
        &mut self,
        attempt_id: &str,
        outcome: &str,
        usage: Option<AdviceUsage>,
        latency_ms: u64,
        cache_json: Option<&str>,
        snapshot: &AdviceSnapshot,
    ) -> Result<bool, MemoryError> {
        finish(
            self,
            Completion {
                attempt_id,
                outcome,
                usage,
                latency_ms,
                cache_json,
            },
            Some(snapshot),
        )
    }

    /// Record a pre-reservation fallback only. Admission/finish count themselves.
    pub fn record_daily_memory_advice_fallback(&mut self, reason: &str) -> Result<(), MemoryError> {
        validate_reason(reason, false)?;
        increment(self.conn(), &Utc::now().date_naive().to_string(), reason)
    }

    /// Read aggregate accounting; unfinished reservations remain unknown usage.
    pub fn daily_memory_advice_stats(&self, day: &str) -> Result<AdviceStats, MemoryError> {
        if day.len() != 10
            || NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map_or(true, |parsed| parsed.to_string() != day)
        {
            return Err(invalid("advice stats day must be YYYY-MM-DD UTC"));
        }
        let tx = self.conn().unchecked_transaction()?;
        let mut stats = AdviceStats::default();
        {
            let mut statement = tx.prepare(
                "SELECT outcome,input_tokens,output_tokens,latency_ms,finished_at_ms
                 FROM daily_memory_advice_attempts WHERE day=?1",
            )?;
            let mut rows = statement.query([day])?;
            while let Some(row) = rows.next()? {
                stats.reserved += 1;
                let outcome: Option<String> = row.get(0)?;
                if row.get::<_, Option<i64>>(4)?.is_some() {
                    stats.finished += 1;
                }
                if outcome.as_deref() == Some("success") {
                    stats.succeeded += 1;
                } else if let Some(reason) = outcome {
                    *stats.fallbacks.entry(reason).or_default() += 1;
                }
                if let Some(tokens) = optional_unsigned(row, 1)? {
                    stats.usage_known += 1;
                    stats.input_tokens = stats.input_tokens.saturating_add(tokens);
                    stats.output_tokens = stats.output_tokens.saturating_add(unsigned(row, 2)?);
                }
                stats.latency_ms = stats
                    .latency_ms
                    .saturating_add(optional_unsigned(row, 3)?.unwrap_or(0));
            }
        }
        stats.usage_unknown = stats.reserved - stats.usage_known;
        {
            let mut statement =
                tx.prepare("SELECT reason,count FROM daily_memory_advice_counters WHERE day=?1")?;
            let mut rows = statement.query([day])?;
            while let Some(row) = rows.next()? {
                let reason: String = row.get(0)?;
                let count = unsigned(row, 1)?;
                match reason.as_str() {
                    "cache_hit" => stats.cache_hits = count,
                    "budget_exhausted" => stats.budget_exhausted = count,
                    "busy" => stats.busy = count,
                    _ => {}
                }
                if reason != "cache_hit" {
                    let total = stats.fallbacks.entry(reason).or_default();
                    *total = total.saturating_add(count);
                }
            }
        }
        tx.commit()?;
        Ok(stats)
    }
}

struct Completion<'a> {
    attempt_id: &'a str,
    outcome: &'a str,
    usage: Option<AdviceUsage>,
    latency_ms: u64,
    cache_json: Option<&'a str>,
}

struct AttemptState {
    cache_key: String,
    model: String,
    schema_version: String,
    finished_at_ms: Option<i64>,
    outcome: Option<String>,
}

fn finish(
    store: &mut MetadataStore,
    completion: Completion<'_>,
    snapshot: Option<&AdviceSnapshot>,
) -> Result<bool, MemoryError> {
    let Completion {
        attempt_id,
        mut outcome,
        usage,
        latency_ms,
        mut cache_json,
    } = completion;
    bounded("advice attempt id", attempt_id, 256)?;
    validate_reason(outcome, true)?;
    if let Some(data) = cache_json {
        if outcome != "success" {
            return Err(invalid("only successful advice may populate cache"));
        }
        validate_cache(data)?;
    }
    let input = usage.map(|u| integer(u.input_tokens)).transpose()?;
    let output = usage.map(|u| integer(u.output_tokens)).transpose()?;
    let latency = integer(latency_ms)?;
    let tx = store
        .conn_mut()
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = if let Some(snapshot) = snapshot {
        let scope = snapshot.scope.key()?;
        let receipt = &snapshot.receipt;
        bounded("advice memory id", &receipt.id, 256)?;
        revision(&tx, &scope)? == receipt.scope_revision
            && read_record(&tx, &scope, &receipt.id)?.is_some_and(|record| {
                record.version == receipt.version
                    && record.event_id == receipt.event_id
                    && record.content_hash == receipt.content_hash
                    && !record.stale
                    && matches!(
                        record.status,
                        MemoryStatus::Accepted | MemoryStatus::Proposed
                    )
            })
    } else {
        true
    };
    let prior: Option<AttemptState> = tx
        .query_row(
            "SELECT cache_key,model,schema_version,finished_at_ms,outcome
         FROM daily_memory_advice_attempts WHERE attempt_id=?1",
            [attempt_id],
            |row| {
                Ok(AttemptState {
                    cache_key: row.get(0)?,
                    model: row.get(1)?,
                    schema_version: row.get(2)?,
                    finished_at_ms: row.get(3)?,
                    outcome: row.get(4)?,
                })
            },
        )
        .optional()?;
    let AttemptState {
        cache_key: key,
        model,
        schema_version: schema,
        finished_at_ms: finished,
        outcome: prior_outcome,
    } = prior.ok_or(MemoryError::NotFound)?;
    if finished.is_some() {
        return Ok(current && prior_outcome.as_deref() != Some("stale"));
    }
    if !current {
        outcome = "stale";
        cache_json = None;
    }
    let now = Utc::now().timestamp_millis();
    tx.execute(
        "UPDATE daily_memory_advice_attempts SET finished_at_ms=?2,outcome=?3,
         input_tokens=?4,output_tokens=?5,latency_ms=?6 WHERE attempt_id=?1",
        params![attempt_id, now, outcome, input, output, latency],
    )?;
    if let Some(data) = cache_json {
        tx.execute(
            "INSERT INTO daily_memory_advice_cache(cache_key,model,schema_version,data,used_at_ms)
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT(cache_key,model,schema_version)
             DO UPDATE SET data=excluded.data,used_at_ms=excluded.used_at_ms",
            params![key, model, schema, data, now],
        )?;
        tx.execute(
            "DELETE FROM daily_memory_advice_cache WHERE rowid NOT IN
             (SELECT rowid FROM daily_memory_advice_cache
              ORDER BY used_at_ms DESC,rowid DESC LIMIT ?1)",
            [CACHE_LIMIT as i64],
        )?;
    }
    tx.commit()?;
    Ok(current)
}

fn reserve_at(
    store: &mut MetadataStore,
    key: &str,
    model: &str,
    schema: &str,
    now: DateTime<Utc>,
) -> Result<AdviceAdmission, MemoryError> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(
            "advice cache key must contain exactly 64 hex digits",
        ));
    }
    for (name, value, max) in [("model", model, 128), ("schema version", schema, 128)] {
        bounded(name, value, max)?;
        if value.chars().any(char::is_control) {
            return Err(invalid(
                "advice model/schema cannot contain control characters",
            ));
        }
    }
    let day = now.date_naive().to_string();
    let millis = now.timestamp_millis();
    let tx = store
        .conn_mut()
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let cached: Option<String> = tx
        .query_row(
            "SELECT data FROM daily_memory_advice_cache
             WHERE cache_key=?1 AND model=?2 AND schema_version=?3",
            params![key, model, schema],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(data) = cached {
        validate_cache(&data)?;
        tx.execute(
            "UPDATE daily_memory_advice_cache SET used_at_ms=?4
             WHERE cache_key=?1 AND model=?2 AND schema_version=?3",
            params![key, model, schema, millis],
        )?;
        increment(&tx, &day, "cache_hit")?;
        tx.commit()?;
        return Ok(AdviceAdmission::Cached(data));
    }
    let busy: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM daily_memory_advice_attempts
         WHERE finished_at_ms IS NULL AND lease_until_ms>?1)",
        [millis],
        |row| row.get(0),
    )?;
    if busy {
        increment(&tx, &day, "busy")?;
        tx.commit()?;
        return Ok(AdviceAdmission::Busy);
    }
    let charged: u64 = tx.query_row(
        "SELECT count(*) FROM daily_memory_advice_attempts WHERE day=?1",
        [&day],
        |row| unsigned(row, 0),
    )?;
    if charged >= DAILY_LIMIT {
        increment(&tx, &day, "budget_exhausted")?;
        tx.commit()?;
        return Ok(AdviceAdmission::BudgetExhausted);
    }
    let reservation = AdviceReservation {
        attempt_id: cortex_core::event_id(),
    };
    tx.execute(
        "INSERT INTO daily_memory_advice_attempts
         (attempt_id,day,cache_key,model,schema_version,reserved_at_ms,lease_until_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            reservation.attempt_id,
            day,
            key,
            model,
            schema,
            millis,
            millis + LEASE_MS
        ],
    )?;
    tx.commit()?;
    Ok(AdviceAdmission::Reserved(reservation))
}

fn integer(value: u64) -> Result<i64, MemoryError> {
    i64::try_from(value).map_err(|_| invalid("advice accounting exceeds SQLite integer range"))
}

fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn optional_unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    let value: Option<i64> = row.get(index)?;
    value
        .map(|number| {
            u64::try_from(number)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, number))
        })
        .transpose()
}

fn validate_cache(data: &str) -> Result<(), MemoryError> {
    if data.len() > CACHE_BYTES
        || !serde_json::from_str::<serde_json::Value>(data)
            .map_err(|_| invalid("advice cache contains invalid JSON"))?
            .is_object()
    {
        return Err(invalid("advice cache must be a JSON object at most 16 KiB"));
    }
    Ok(())
}

fn increment(conn: &Connection, day: &str, reason: &str) -> Result<(), MemoryError> {
    conn.execute(
        "INSERT INTO daily_memory_advice_counters(day,reason,count) VALUES(?1,?2,1)
         ON CONFLICT(day,reason) DO UPDATE SET count=count+1",
        params![day, reason],
    )?;
    Ok(())
}

fn validate_reason(reason: &str, success: bool) -> Result<(), MemoryError> {
    if (success && reason == "success")
        || matches!(
            reason,
            "disabled"
                | "missing_key"
                | "unsafe_state"
                | "state_too_large"
                | "unavailable"
                | "response_too_large"
                | "invalid_response"
                | "timeout"
                | "stale"
                | "storage_unavailable"
                | "busy"
                | "budget_exhausted"
        )
    {
        Ok(())
    } else {
        Err(invalid("unsupported advice outcome/fallback reason"))
    }
}

#[cfg(test)]
mod tests;
