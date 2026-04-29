//! Time-bucketed series helpers for the dashboard overview surface.
//!
//! Splitting these out of `dashboard.rs` keeps the route handlers
//! compact and makes the bucketing logic unit-testable in isolation.
//! Phase2g §1.6 — `pre_thinking_p95_ms` and `classifier_cost_usd_today`
//! land here alongside the existing `events_per_min` /
//! `violations_7d_daily` helpers re-exported from the parent module.

use chrono::{DateTime, Duration as ChronoDuration, Timelike, Utc};
use cortex_storage::{hour_bucket_rfc3339, MetadataStore};

use crate::lanes::LaneHit;

/// Length of the `classifier_cost_usd_today` series. 24 hour-wide
/// buckets covering the rolling day so the GUI renders a daily ribbon.
/// Every bucket is `0.0` until the spec-05 classifier worker stamps
/// per-event cost on the lane.
pub const CLASSIFIER_COST_BUCKETS: usize = 24;

/// Bucket `hits` into `buckets` minute-wide slots ending at `now_ms`.
/// Slot 0 is the oldest (`now_ms - buckets * 60_000`), slot
/// `buckets - 1` is the most recent. Hits older than the window are
/// ignored; hits in the future (clock-skew) collapse into the last
/// slot.
pub fn bucket_per_minute(hits: &[LaneHit], now_ms: i64, buckets: usize) -> Vec<u64> {
    let mut out = vec![0u64; buckets];
    if buckets == 0 {
        return out;
    }
    let span_ms = (buckets as i64) * 60_000;
    let start_ms = now_ms - span_ms;
    for h in hits {
        if h.ts <= 0 || h.ts < start_ms {
            continue;
        }
        let off = (h.ts - start_ms) / 60_000;
        let idx = if off < 0 {
            0
        } else if off >= buckets as i64 {
            buckets - 1
        } else {
            off as usize
        };
        out[idx] += 1;
    }
    out
}

/// Bucket `kind=law_violation` envelopes into `days` day-wide slots
/// ending at `now_ms`. Slot 0 is the oldest, slot `days - 1` is
/// today.
pub fn bucket_violations_per_day(hits: &[LaneHit], now_ms: i64, days: usize) -> Vec<u64> {
    let mut out = vec![0u64; days];
    if days == 0 {
        return out;
    }
    let day_ms: i64 = 86_400_000;
    let span_ms = (days as i64) * day_ms;
    let start_ms = now_ms - span_ms;
    for h in hits {
        if h.symbol.as_deref() != Some("law_violation") {
            continue;
        }
        if h.ts <= 0 || h.ts < start_ms {
            continue;
        }
        let off = (h.ts - start_ms) / day_ms;
        let idx = if off < 0 {
            0
        } else if off >= days as i64 {
            days - 1
        } else {
            off as usize
        };
        out[idx] += 1;
    }
    out
}

/// P95 of `extras["duration_ms"]` per minute bucket. Returns one slot
/// per minute (oldest-first); each slot is `Some(p95)` when at least
/// one envelope landed in that minute carrying a duration stamp,
/// `None` otherwise so the front-end Sparkline draws a gap.
///
/// Until spec-12 lands turn-level latency, only ToolCall + AgentCall
/// envelopes carry `duration_ms` — `archive_loader::envelope_to_hit`
/// stamps that field on extras. Turns alone won't populate the
/// series; that's surfaced honestly in the response (the matching
/// tile renders "—" for empty buckets).
pub fn bucket_p95_duration_per_minute(
    hits: &[LaneHit],
    now_ms: i64,
    buckets: usize,
) -> Vec<Option<u64>> {
    let mut out: Vec<Option<u64>> = vec![None; buckets];
    if buckets == 0 {
        return out;
    }
    let span_ms = (buckets as i64) * 60_000;
    let start_ms = now_ms - span_ms;
    let mut samples: Vec<Vec<u64>> = vec![Vec::new(); buckets];
    for h in hits {
        if h.ts <= 0 || h.ts < start_ms {
            continue;
        }
        let off = (h.ts - start_ms) / 60_000;
        let idx = if off < 0 {
            0
        } else if off >= buckets as i64 {
            buckets - 1
        } else {
            off as usize
        };
        if let Some(d) = h
            .extras
            .get("duration_ms")
            .and_then(|v| v.as_u64())
        {
            samples[idx].push(d);
        }
    }
    for (i, mut bucket) in samples.into_iter().enumerate() {
        if bucket.is_empty() {
            continue;
        }
        bucket.sort_unstable();
        out[i] = Some(percentile(&bucket, 95));
    }
    out
}

/// Linear-interpolated percentile of a sorted slice. Returns 0 for
/// empty input so callers can guard with `is_empty` first; the
/// public bucketing helper does that already.
fn percentile(sorted: &[u64], p: u8) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let p = p.min(100) as f64 / 100.0;
    let rank = p * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    let frac = rank - lo as f64;
    let interpolated = sorted[lo] as f64 + (sorted[hi] - sorted[lo]) as f64 * frac;
    interpolated.round() as u64
}

/// 24 hourly slots for `classifier_cost_usd_today`, all zero. The
/// `classifier_cost_unavailable_until_spec05` flag at the top of
/// `OverviewBody` tells the front-end the zeros are honest "we
/// don't have this signal yet" data rather than fabricated values.
/// The series shape exists so the wire contract stays stable across
/// the spec-05 cut-over — clients that already render it will see a
/// flat zero ribbon today and real numbers once the classifier
/// worker ships.
pub fn classifier_cost_zeros() -> Vec<f64> {
    vec![0.0; CLASSIFIER_COST_BUCKETS]
}

/// Read the rolling-24h classifier cost ribbon from the metadata
/// store. Returns `(series, is_stub)` where `is_stub = true` means
/// "no spend data found in the window" — the dashboard surfaces that
/// flag verbatim so the GUI keeps printing "—" instead of a
/// dishonest `$0.00` until the classifier-worker has booked at least
/// one classification.
///
/// `now` defines the upper edge of the window; the returned series
/// has [`CLASSIFIER_COST_BUCKETS`] hour-wide slots, oldest-first,
/// each carrying the USD spend for that UTC hour bucket.
pub fn read_classifier_cost_24h(store: &MetadataStore, now: DateTime<Utc>) -> (Vec<f64>, bool) {
    let mut out = vec![0.0_f64; CLASSIFIER_COST_BUCKETS];
    // Span the 24 buckets ending at the hour that contains `now`.
    let now_hour_truncated = now
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now);
    let oldest = now_hour_truncated - ChronoDuration::hours((CLASSIFIER_COST_BUCKETS as i64) - 1);
    let since_key = hour_bucket_rfc3339(oldest);
    let until_key = hour_bucket_rfc3339(now_hour_truncated);
    let rows = match store.classifier_spend_hourly_window(&since_key, &until_key) {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(error = %err, "classifier_spend_hourly_window failed; ribbon stays stubbed");
            return (out, true);
        }
    };
    if rows.is_empty() {
        return (out, true);
    }
    for row in rows {
        // Map the bucket key back to an offset 0..24. Lexicographic
        // ordering of the canonical RFC3339 hour key matches calendar
        // order, so a parse-then-diff is the reliable path.
        let parsed = match DateTime::parse_from_rfc3339(&row.hour) {
            Ok(t) => t.with_timezone(&Utc),
            Err(err) => {
                tracing::warn!(error = %err, hour = %row.hour, "skipping malformed hour key");
                continue;
            }
        };
        let delta_hours = (parsed - oldest).num_hours();
        if delta_hours < 0 || delta_hours >= CLASSIFIER_COST_BUCKETS as i64 {
            continue;
        }
        out[delta_hours as usize] = (row.spend.est_usd_cents as f64) / 100.0;
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn hit_with_duration(ts: i64, duration_ms: u64) -> LaneHit {
        let mut extras: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        extras.insert(
            "duration_ms".to_string(),
            serde_json::Value::Number(serde_json::Number::from(duration_ms)),
        );
        LaneHit {
            doc_id: format!("archive|{ts}"),
            text: String::new(),
            repo: None,
            path: None,
            symbol: Some("tool_call:Edit".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras,
        }
    }

    fn turn_hit(ts: i64) -> LaneHit {
        LaneHit {
            doc_id: format!("archive|t{ts}"),
            text: String::new(),
            repo: None,
            path: None,
            symbol: Some("turn".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras: BTreeMap::new(),
        }
    }

    fn violation_hit(ts: i64) -> LaneHit {
        LaneHit {
            doc_id: format!("archive|v{ts}"),
            text: String::new(),
            repo: None,
            path: None,
            symbol: Some("law_violation".to_string()),
            content_hash: None,
            score: 1.0,
            ts,
            severity: None,
            extras: BTreeMap::new(),
        }
    }

    #[test]
    fn bucket_per_minute_emits_oldest_first_and_drops_out_of_range() {
        // now = minute 25; window = 20 buckets ending at `now`, so
        // start_ms = minute 5. Two hits land in bucket 0
        // (between minute 5 and minute 6), one lands in bucket 15
        // (between minute 20 and minute 21), one is older than the
        // window and is dropped, one is in the future and collapses
        // into the last bucket.
        let now: i64 = 25 * 60_000;
        let hits = vec![
            turn_hit(now - 19 * 60_000 - 30_000),
            turn_hit(now - 19 * 60_000 - 10_000),
            turn_hit(now - 5 * 60_000),
            turn_hit(now - 25 * 60_000),
            turn_hit(now + 60_000),
        ];
        let series = bucket_per_minute(&hits, now, 20);
        assert_eq!(series.len(), 20);
        assert_eq!(series[0], 2);
        assert_eq!(series[15], 1);
        assert_eq!(series[19], 1);
        assert_eq!(series.iter().sum::<u64>(), 4);
    }

    #[test]
    fn bucket_violations_per_day_only_counts_law_violation_envelopes() {
        let now: i64 = 7 * 86_400_000;
        let hits = vec![
            violation_hit(now - 6 * 86_400_000 - 1000),
            violation_hit(now - 1000),
            turn_hit(now - 1000),
        ];
        let series = bucket_violations_per_day(&hits, now, 7);
        assert_eq!(series.len(), 7);
        assert_eq!(series[0], 1);
        assert_eq!(series[6], 1);
        assert_eq!(series.iter().sum::<u64>(), 2);
    }

    #[test]
    fn p95_per_minute_is_none_when_no_durations_landed() {
        let now: i64 = 20 * 60_000;
        let hits = vec![turn_hit(now - 30_000)];
        let series = bucket_p95_duration_per_minute(&hits, now, 20);
        assert_eq!(series.len(), 20);
        assert!(series.iter().all(Option::is_none));
    }

    #[test]
    fn p95_per_minute_picks_the_p95_across_samples_in_each_bucket() {
        let now: i64 = 5 * 60_000;
        let hits: Vec<LaneHit> = (1..=100u64)
            .map(|i| hit_with_duration(now - 30_000, i))
            .collect();
        let series = bucket_p95_duration_per_minute(&hits, now, 5);
        for (i, slot) in series.iter().enumerate().take(4) {
            assert!(slot.is_none(), "bucket {i} expected None");
        }
        assert_eq!(series[4], Some(95));
    }

    #[test]
    fn classifier_cost_zeros_is_24_zero_slots() {
        let cost = classifier_cost_zeros();
        assert_eq!(cost.len(), CLASSIFIER_COST_BUCKETS);
        assert!(cost.iter().all(|v| *v == 0.0));
    }
}
