//! Phase9j synthetic corpus generator.
//!
//! Produces a deterministic 1k-event mix that exercises every
//! retention boundary the canary asserts on. The generator is pure
//! Rust + no I/O — callers project the resulting `SynthEnvelope`s
//! into whichever in-memory backend they need (Vectorizer, Meili,
//! CAS, MetadataStore, archive on-disk).

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use cortex_workers::retention::pii_enforce::PiiRisk;
use cortex_workers::retention::{RecordRef, SweepKind, Tier};

/// Canonical envelope kinds the corpus mixes. Mirrors the spec-04
/// kind set without coupling the test to that crate (kept as a
/// string so the corpus can grow without recompilation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusKind {
    Turn,
    ToolCall,
    Decision,
    Analysis,
    Memory,
}

impl CorpusKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CorpusKind::Turn => "turn",
            CorpusKind::ToolCall => "tool_call",
            CorpusKind::Decision => "decision",
            CorpusKind::Analysis => "analysis",
            CorpusKind::Memory => "memory",
        }
    }
    /// Whether this kind participates in the tier-sweep pipeline
    /// (Vectorizer collections cortex.<kind>.{fp32,pq}).
    pub fn sweep_kind(self) -> Option<SweepKind> {
        match self {
            CorpusKind::Turn => Some(SweepKind::Turn),
            CorpusKind::ToolCall => Some(SweepKind::ToolCall),
            // decisions / analyses / memory live in their own
            // single-tier collections that the sweep does NOT
            // demote — see spec 02 §"Per-collection tier
            // strategy".
            _ => None,
        }
    }
}

/// One synthetic envelope. Carries every field the various
/// retention sweeps need to filter on.
#[derive(Debug, Clone)]
pub struct SynthEnvelope {
    pub event_id: String,
    pub kind: CorpusKind,
    pub repo: &'static str,
    pub session_id: &'static str,
    pub occurred_at: DateTime<Utc>,
    pub age_bucket: AgeBucket,
    pub pii_risk: Option<PiiRisk>,
    pub body: String,
    pub topic: &'static str,
}

/// Boundary buckets the spec promises every canary run covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeBucket {
    /// Now — fresh data. Stays in fp32 / hourly archive.
    Now,
    /// `now - 15 d` — fresh-ish, still in fp32, still hourly.
    D15,
    /// `now - 31 d` — eligible for FP32 → PQ + bootstrap_jobs roll.
    D31,
    /// `now - 91 d` — eligible for archive hourly→daily + Meili
    /// prune + PII medium.
    D91,
    /// `now - 366 d` — eligible for PQ → Binary + archive daily→
    /// monthly + sessions/spend roll.
    D366,
    /// `now - 1100 d` — eligible for the 3-y archive drop.
    D1100,
}

impl AgeBucket {
    pub fn offset_days(self) -> i64 {
        match self {
            AgeBucket::Now => 0,
            AgeBucket::D15 => 15,
            AgeBucket::D31 => 31,
            AgeBucket::D91 => 91,
            AgeBucket::D366 => 366,
            AgeBucket::D1100 => 1_100,
        }
    }
    pub const ALL: [AgeBucket; 6] = [
        AgeBucket::Now,
        AgeBucket::D15,
        AgeBucket::D31,
        AgeBucket::D91,
        AgeBucket::D366,
        AgeBucket::D1100,
    ];
}

/// The deterministic 1k corpus the canary feeds through every
/// retention stage. The exact mix:
///
/// - 600 turns, 250 tool_calls, 50 decisions, 50 analyses, 50 memory
/// - distributed across the 6 boundary age buckets
/// - PII tags: 60 % null, 25 % low, 10 % medium, 5 % high
///
/// The function is deterministic (no RNG) so a CI re-run produces
/// byte-identical envelopes — assertions can compare exact counts.
pub fn build(now: DateTime<Utc>) -> Vec<SynthEnvelope> {
    let mix: &[(CorpusKind, usize)] = &[
        (CorpusKind::Turn, 600),
        (CorpusKind::ToolCall, 250),
        (CorpusKind::Decision, 50),
        (CorpusKind::Analysis, 50),
        (CorpusKind::Memory, 50),
    ];
    let pii_pattern: &[Option<PiiRisk>] = &[
        // 100-row pattern repeated for each kind: 60 null / 25 low /
        // 10 medium / 5 high. Indexing by (counter % 100) keeps the
        // distribution exact across kind sizes.
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Low),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::Medium),
        Some(PiiRisk::High),
        Some(PiiRisk::High),
        Some(PiiRisk::High),
        Some(PiiRisk::High),
        Some(PiiRisk::High),
    ];
    assert_eq!(pii_pattern.len(), 100);

    let mut out = Vec::with_capacity(1_000);
    let buckets = AgeBucket::ALL;
    // Single global counter so the PII pattern's 60/25/10/5
    // distribution is exact across the full corpus, not per-kind
    // (a per-kind counter would land every 50-row kind entirely in
    // the 60-null prefix of the pattern).
    let mut counter: u64 = 0;
    for &(kind, count) in mix {
        for i in 0..count {
            let bucket = buckets[i % buckets.len()];
            let occurred = now - Duration::days(bucket.offset_days());
            let pii = pii_pattern[(counter as usize) % pii_pattern.len()];
            let event_id = format!("{}-{:05}", kind.as_str(), counter);
            counter += 1;
            out.push(SynthEnvelope {
                event_id: event_id.clone(),
                kind,
                repo: "Cortex",
                session_id: "01CANARY-SESSION",
                occurred_at: occurred,
                age_bucket: bucket,
                pii_risk: pii,
                // Unique body per envelope so the CAS store keeps one
                // blob per envelope (otherwise identical bodies
                // dedup to one hash and the orphan-seed loop fights
                // refcount bumps on subsequent puts).
                body: format!(
                    "[{kind}/{bucket:?}/{event_id}] synthetic body for {topic}",
                    kind = kind.as_str(),
                    bucket = bucket,
                    topic = "retention",
                ),
                topic: "retention",
            });
        }
    }
    assert_eq!(out.len(), 1_000);
    out
}

/// Project a `SynthEnvelope` into the [`RecordRef`] shape the tier
/// sweep consumes.
pub fn to_record_ref(env: &SynthEnvelope) -> RecordRef {
    RecordRef {
        event_id: env.event_id.clone(),
        kind: env.kind.as_str().to_string(),
        occurred_at: env.occurred_at,
        bytes: env.body.as_bytes().to_vec(),
    }
}

/// Compute the canonical fp32 / pq tier label for a given age
/// bucket. Used by the canary to decide which seeded collection a
/// record starts in: anything `< 30 d` belongs in FP32, `30..=365 d`
/// in PQ, `> 365 d` in Binary already (so the sweep just verifies
/// it stays put).
pub fn starting_tier(env: &SynthEnvelope) -> Tier {
    match env.age_bucket {
        AgeBucket::Now | AgeBucket::D15 => Tier::Fp32,
        AgeBucket::D31 | AgeBucket::D91 => Tier::Fp32,
        AgeBucket::D366 => Tier::Pq,
        AgeBucket::D1100 => Tier::Binary,
    }
}

/// Plant a `.corrupted` Parquet artifact directly under the events
/// root. The canary asserts that the rollup pre-flight quarantines
/// it before any compaction starts.
pub fn plant_corrupted_artifact(events_root: &Path) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(events_root)?;
    let path = events_root.join("raw-00000.parquet.corrupted");
    std::fs::write(&path, b"intentionally-not-zstd-not-parquet")?;
    Ok(path)
}

/// Write one zstd-NDJSON archive file representing `envelopes` at
/// `<root>/year=YYYY/month=MM/day=DD/hour=HH/<stream>-00000.parquet`.
/// The compactor reads zstd-NDJSON (the actual on-disk format the
/// archive uses today, before the binary Parquet writer lands —
/// see `parquet_rollup::read_source_file`). One file per partition
/// is enough to drive the rollup deterministically.
pub fn write_archive_file(
    archive_root: &Path,
    ts: DateTime<Utc>,
    stream: &str,
    envelopes: &[&SynthEnvelope],
) -> std::io::Result<std::path::PathBuf> {
    use chrono::Datelike;
    use chrono::Timelike;
    let dir = archive_root
        .join("events")
        .join(format!("year={:04}", ts.year()))
        .join(format!("month={:02}", ts.month()))
        .join(format!("day={:02}", ts.day()))
        .join(format!("hour={:02}", ts.hour()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{stream}-00000.parquet"));
    let file = std::fs::File::create(&path)?;
    let mut encoder = zstd::stream::write::Encoder::new(file, 6)?;
    use std::io::Write;
    for env in envelopes {
        let line = serde_json::json!({
            "event_id": env.event_id,
            "kind": env.kind.as_str(),
            "repo": env.repo,
            "session_id": env.session_id,
            "occurred_at_ms": env.occurred_at.timestamp_millis(),
            "pii_risk": env.pii_risk.map(|r| r.as_str()),
            "body": env.body,
        });
        let s = line.to_string();
        encoder.write_all(s.as_bytes())?;
        encoder.write_all(b"\n")?;
    }
    let inner = encoder.finish()?;
    inner.sync_all()?;
    Ok(path)
}
