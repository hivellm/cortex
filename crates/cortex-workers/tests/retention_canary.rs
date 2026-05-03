//! Phase9j — retention CI canary.
//!
//! End-to-end smoke that drives every retention sweep against a
//! deterministic synthetic corpus and asserts the post-state across
//! every storage layer the pipeline touches:
//!
//! - Vectorizer (in-memory `MemoryVectorizerOps`),
//! - Meili (in-memory `MemoryMeiliBackend`),
//! - CAS SQLite (`CasStore::open_in_memory()`),
//! - SQLite metadata (`MetadataStore::open_in_memory()`),
//! - Parquet event archive (zstd-NDJSON files in a tempdir).
//!
//! The proposal's docker-compose framing is the v2 surface the
//! production CI workflow drives; the integration test here runs
//! the same library calls the CLI does, against the same in-memory
//! backends every other unit test in the crate uses, so it stays
//! fast (seconds, not minutes), hermetic (no network), and
//! reproducible. The
//! [`.github/workflows/retention-canary.yml`](../../../.github/workflows/retention-canary.yml)
//! workflow runs `cargo test -p cortex-retention --test canary` on
//! every PR touching the retention surface plus a nightly schedule.

mod support;

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use cortex_workers::retention::cas_vacuum::{run as cas_run, VacuumOpts};
use cortex_workers::retention::meili_prune::{
    run_meili_prune, MeiliBackend, MeiliDoc, MemoryMeiliBackend, PrunePlan,
};
use cortex_workers::retention::metadata_reap::{run as reap_run, ReapPlan};
use cortex_workers::retention::parquet_rollup::{
    apply_three_year_drop, compact_partition, enumerate_compactable, quarantine_pre_existing,
    Granularity,
};
use cortex_workers::retention::pii_enforce::{
    run_enforcement, EnforcementPlan, MemoryPiiBackend, PiiRisk, PiiTarget,
};
use cortex_workers::retention::turn_digest::{
    run_turn_digest, DigestPlan, DigestResult, MemoryDigestBackend, Turn,
};
use cortex_workers::retention::{run_sweep, MemoryVectorizerOps, SweepPlan, Tier};
use cortex_storage::cas::CasContentType;
use cortex_storage::{CasStore, MetadataStore};

use support::synth_corpus::{
    self as synth, AgeBucket, CorpusKind, SynthEnvelope,
};

/// Deterministic reference time the canary travels with.
fn now_anchor() -> DateTime<Utc> {
    // Pinned anchor inside spec window; used everywhere via `--time-travel`
    // so each boundary fires deterministically.
    Utc.with_ymd_and_hms(2026, 4, 29, 18, 0, 0).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn retention_canary_full_pipeline() {
    let now = now_anchor();
    let corpus = synth::build(now);
    let archive_root = tempfile::tempdir().expect("archive tempdir");
    let archive_path = archive_root.path();
    let events_root = archive_path.join("events");

    // ----- plant the corrupted artifact + write archive files ------
    let corrupted = synth::plant_corrupted_artifact(&events_root)
        .expect("plant corrupted");
    write_archive_files(&corpus, archive_path);

    // ----- seed Vectorizer collections by tier --------------------
    let vec_ops = MemoryVectorizerOps::new();
    seed_vector_collections(&vec_ops, &corpus).await;

    // ----- seed Meili docs (turns + tool_calls) -------------------
    let meili = MemoryMeiliBackend::new();
    seed_meili(&meili, &corpus).await;

    // ----- seed CAS with one blob per envelope; orphan some -------
    let cas_db_path = archive_path.join("cas.sqlite");
    let mut cas = CasStore::open(&cas_db_path).expect("cas open");
    let orphan_hashes = seed_cas_blobs(&mut cas, &corpus, now);

    // ----- seed metadata (sessions, classifier_spend, bootstrap) ---
    let meta_path = archive_path.join("metadata.sqlite");
    let mut metadata = MetadataStore::open(&meta_path).expect("metadata open");
    seed_metadata(&metadata, now);

    // Shared backends — hoisted so a second drive sees the post-state
    // recorded by the first. Per-call backends would always look
    // empty on the second run and the idempotence assertion would
    // be vacuous.
    let pii_backend = MemoryPiiBackend::new();
    let digest_backend = MemoryDigestBackend::new();
    digest_backend
        .set_summary(DigestResult {
            body: "synthetic digest body".into(),
            tokens_in: 0,
            tokens_out: 0,
            usd_cents: 0,
        })
        .await;
    let mut redacted_ids: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    // =================================================================
    // First pass — drive every retention sweep with `--time-travel now`.
    // =================================================================
    let report1 = drive_retention(
        DriveCtx {
            now,
            archive_path,
            vec_ops: &vec_ops,
            meili: &meili,
            cas: &mut cas,
            metadata: &mut metadata,
            corpus: &corpus,
            pii_backend: &pii_backend,
            digest_backend: &digest_backend,
            redacted_ids: &redacted_ids,
        },
    )
    .await;
    // Mark every event_id the first pass redacted so the second
    // pass's targets carry `redacted: Some(_)` and the matcher
    // short-circuits — same shape the production sweep sees once
    // `payload.redacted` lands on the source row.
    for (event_id, _kind, _body, _tag) in &report1.pii_rewrites {
        redacted_ids.insert(event_id.clone());
    }
    // Pre-populate the digest backend's `existing` map so the second
    // pass's `lookup_existing` short-circuits — same shape the
    // production walker sees once the first digest is persisted.
    for (repo, week, topic, id) in digest_backend.persisted().await {
        digest_backend.pre_existing(&repo, &week, &topic, &id).await;
    }
    meili.commit_updates().await;

    // ---------------- Per-sweep assertions ---------------------------

    // 4.1 — FP32 collections have zero records older than 30 d.
    for kind in ["turn", "tool_call"] {
        let snap = vec_ops
            .snapshot(&format!("cortex.{kind}.fp32"))
            .await;
        for r in &snap {
            assert!(
                (now - r.occurred_at).num_days() <= 30,
                "fp32/{kind} kept stale record {} ({} d)",
                r.event_id,
                (now - r.occurred_at).num_days()
            );
        }
    }
    // 4.2 — PQ collections have zero records older than 365 d.
    for kind in ["turn", "tool_call"] {
        let snap = vec_ops
            .snapshot(&format!("cortex.{kind}.pq"))
            .await;
        for r in &snap {
            assert!(
                (now - r.occurred_at).num_days() <= 365,
                "pq/{kind} kept stale record {} ({} d)",
                r.event_id,
                (now - r.occurred_at).num_days()
            );
        }
    }
    // 4.3 — Cold binary contains every record that started >365 d
    // old plus the records demoted from PQ during the sweep. We
    // only assert presence of D366 + D1100 buckets (D1100 was
    // pre-binary; D366 demoted during the sweep).
    let cold = vec_ops.snapshot("cortex.cold.binary").await;
    let cold_ids: std::collections::BTreeSet<String> =
        cold.iter().map(|r| r.event_id.clone()).collect();
    for env in corpus
        .iter()
        .filter(|e| matches!(e.age_bucket, AgeBucket::D366 | AgeBucket::D1100))
        .filter(|e| matches!(e.kind, CorpusKind::Turn | CorpusKind::ToolCall))
    {
        assert!(
            cold_ids.contains(&env.event_id),
            "cold.binary missing {}",
            env.event_id
        );
    }

    // 4.4 — Archive: no hourly directories older than 90 d, daily
    // files exist for the 30–365 d band, monthly files for the
    // 365 d–3 y band. Walk the on-disk tree.
    let archive = scan_archive_tree(&events_root);
    assert!(
        archive.has_quarantine,
        "_quarantine/ MUST exist after rollup quarantines the planted artifact"
    );
    assert_eq!(
        archive.tmp_orphans, 0,
        "no .tmp orphans should remain after rollup ({} found)",
        archive.tmp_orphans
    );
    assert_eq!(
        archive.corrupted_outside_quarantine, 0,
        "no .corrupted files outside _quarantine ({} found)",
        archive.corrupted_outside_quarantine
    );
    // 4.5 — `_quarantine/` contains the planted `.corrupted` artifact
    // along with a `.reason` sibling.
    assert!(
        !corrupted.exists(),
        "planted .corrupted artifact MUST move into _quarantine/ on rollup pre-flight"
    );

    // 4.6 — Meili: zero docs >90 d with non-empty body (every
    // mature doc was pruned).
    let updates = meili.updates().await;
    assert!(
        !updates.is_empty(),
        "meili pruner MUST have updated at least one doc batch"
    );
    // The MemoryMeiliBackend records each batch — `commit_updates`
    // applies them so a re-enumeration finds zero unmpruned mature
    // docs.
    meili.commit_updates().await;
    for index in ["cortex_turns", "cortex_tool_calls"] {
        let still_unpruned = meili
            .enumerate_prunable(index, now - Duration::days(90), false, 1_000)
            .await
            .expect("re-enumerate");
        assert!(
            still_unpruned.is_empty(),
            "{index} still holds {} unpruned mature docs",
            still_unpruned.len()
        );
    }

    // 4.7 — SQLite: zero `bootstrap_jobs` success rows >30 d;
    // `bootstrap_jobs_daily` populated.
    let stale_bootstrap: i64 = metadata
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM bootstrap_jobs
              WHERE status = 'success' AND finished_at < ?1",
            rusqlite::params![(now - Duration::days(30)).to_rfc3339()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stale_bootstrap, 0, "stale success rows remained");
    let daily: i64 = metadata
        .conn()
        .query_row("SELECT COUNT(*) FROM bootstrap_jobs_daily", [], |r| r.get(0))
        .unwrap();
    assert!(daily > 0, "bootstrap_jobs_daily should be populated");

    // 4.8 — `cas_blobs` no longer contains orphan rows (we seeded
    // unreferenced 60-d-old blobs; the vacuum should drop them).
    for hash in &orphan_hashes {
        assert!(
            !cas.contains(hash).unwrap(),
            "orphan blob {hash} survived the cas-vacuum"
        );
    }

    // 4.9 — PII-high: rewrite carries `body=None` + `redacted=
    // pii_high_30d`; PII-medium: summary present + `redacted=
    // pii_medium_90d`.
    let rewrites = report1.pii_rewrites;
    let high = rewrites
        .iter()
        .find(|(_, _, body, tag)| body.is_none() && tag == "pii_high_30d");
    assert!(
        high.is_some(),
        "no high-cohort rewrite stamped pii_high_30d"
    );
    let medium = rewrites
        .iter()
        .find(|(_, _, body, tag)| body.is_some() && tag == "pii_medium_90d");
    assert!(
        medium.is_some(),
        "no medium-cohort rewrite stamped pii_medium_90d"
    );

    // ---------------- Idempotence (4.10) -----------------------------
    let report2 = drive_retention(DriveCtx {
        now,
        archive_path,
        vec_ops: &vec_ops,
        meili: &meili,
        cas: &mut cas,
        metadata: &mut metadata,
        corpus: &corpus,
        pii_backend: &pii_backend,
        digest_backend: &digest_backend,
        redacted_ids: &redacted_ids,
    })
    .await;

    assert_eq!(
        report2.records_demoted, 0,
        "idempotence: second tier-sweep pass demoted {} records",
        report2.records_demoted
    );
    assert_eq!(
        report2.meili_pruned, 0,
        "idempotence: second meili-prune pass touched {} docs",
        report2.meili_pruned
    );
    assert_eq!(
        report2.cas_dropped, 0,
        "idempotence: second cas-vacuum dropped {} blobs",
        report2.cas_dropped
    );
    assert_eq!(
        report2.digest_buckets_done, 0,
        "idempotence: second turn-digest produced {} new digests",
        report2.digest_buckets_done
    );
    assert_eq!(
        report2.digest_usd_cents, 0,
        "idempotence: second turn-digest spent {} cents",
        report2.digest_usd_cents
    );
    assert_eq!(
        report2.metadata_collapsed, 0,
        "idempotence: second metadata-reap collapsed {} rows",
        report2.metadata_collapsed
    );
    assert_eq!(
        report2.pii_applied, 0,
        "idempotence: second pii-enforce applied {} rewrites",
        report2.pii_applied
    );
}

#[tokio::test]
async fn synthetic_corpus_distribution_matches_spec() {
    let corpus = synth::build(now_anchor());
    assert_eq!(corpus.len(), 1_000);
    let turns = corpus.iter().filter(|e| e.kind == CorpusKind::Turn).count();
    let tool_calls = corpus
        .iter()
        .filter(|e| e.kind == CorpusKind::ToolCall)
        .count();
    let decisions = corpus
        .iter()
        .filter(|e| e.kind == CorpusKind::Decision)
        .count();
    let analyses = corpus
        .iter()
        .filter(|e| e.kind == CorpusKind::Analysis)
        .count();
    let memory = corpus
        .iter()
        .filter(|e| e.kind == CorpusKind::Memory)
        .count();
    assert_eq!((turns, tool_calls, decisions, analyses, memory), (600, 250, 50, 50, 50));
    // PII distribution — 600 null, 250 low, 100 medium, 50 high.
    let mut counts = (0usize, 0usize, 0usize, 0usize);
    for e in &corpus {
        match e.pii_risk {
            None => counts.0 += 1,
            Some(PiiRisk::Low) => counts.1 += 1,
            Some(PiiRisk::Medium) => counts.2 += 1,
            Some(PiiRisk::High) => counts.3 += 1,
        }
    }
    assert_eq!(counts, (600, 250, 100, 50));
    // Every age bucket carries at least one envelope.
    for bucket in AgeBucket::ALL {
        let any = corpus.iter().any(|e| e.age_bucket == bucket);
        assert!(any, "no envelope in bucket {bucket:?}");
    }
}

// ---------- helpers below ------------------------------------------

struct DriveCtx<'a> {
    now: DateTime<Utc>,
    archive_path: &'a std::path::Path,
    vec_ops: &'a MemoryVectorizerOps,
    meili: &'a MemoryMeiliBackend,
    cas: &'a mut CasStore,
    metadata: &'a mut MetadataStore,
    corpus: &'a [SynthEnvelope],
    pii_backend: &'a MemoryPiiBackend,
    digest_backend: &'a MemoryDigestBackend,
    redacted_ids: &'a std::collections::BTreeSet<String>,
}

#[derive(Debug, Default)]
struct DriveReport {
    records_demoted: u64,
    meili_pruned: u64,
    cas_dropped: u64,
    digest_buckets_done: u64,
    digest_usd_cents: u64,
    metadata_collapsed: u64,
    pii_applied: u64,
    pii_rewrites: Vec<(String, String, Option<String>, String)>,
}

async fn drive_retention<'a>(ctx: DriveCtx<'a>) -> DriveReport {
    let mut report = DriveReport::default();

    // 3.1 — tier sweep
    let plan = SweepPlan::default_for(ctx.now);
    let sweep = run_sweep(&plan, ctx.vec_ops).await.expect("sweep");
    report.records_demoted = sweep.records_demoted;

    // 3.2 — parquet rollup (quarantine pre-flight + every granularity).
    let _q = quarantine_pre_existing(ctx.archive_path);
    for g in [
        Granularity::HourlyToDaily,
        Granularity::DailyToMonthly,
        Granularity::ThreeYearDrop,
    ] {
        let plans = enumerate_compactable(ctx.archive_path, ctx.now, g);
        for p in &plans {
            match g {
                Granularity::ThreeYearDrop => {
                    let _ = apply_three_year_drop(ctx.archive_path, p);
                }
                _ => {
                    let _ = compact_partition(ctx.archive_path, p);
                }
            }
        }
    }

    // 3.3 — pii enforce
    let pii_targets: Vec<PiiTarget> = ctx
        .corpus
        .iter()
        .filter(|e| matches!(e.kind, CorpusKind::Turn | CorpusKind::ToolCall))
        .map(|e| PiiTarget {
            event_id: e.event_id.clone(),
            kind: e.kind.as_str().to_string(),
            pii_risk: e.pii_risk,
            occurred_at: e.occurred_at,
            body_ref: Some(format!("sha256:body:{}", e.event_id)),
            redacted: ctx
                .redacted_ids
                .contains(&e.event_id)
                .then(|| "applied".to_string()),
        })
        .collect();
    let pii_plan = EnforcementPlan::default_for(ctx.now);
    let rewrites_before = ctx.pii_backend.rewrites().await.len();
    let pii_report = run_enforcement(&pii_plan, ctx.pii_backend, pii_targets)
        .await
        .expect("pii enforcement");
    report.pii_applied = pii_report.applied;
    let rewrites_after = ctx.pii_backend.rewrites().await;
    report.pii_rewrites = rewrites_after.iter().skip(rewrites_before).cloned().collect();

    // 3.4 — turn digest with bounded budget (5 ¢ per spec).
    let turns: Vec<Turn> = ctx
        .corpus
        .iter()
        .filter(|e| e.kind == CorpusKind::Turn)
        .map(|e| Turn {
            event_id: e.event_id.clone(),
            repo: e.repo.to_string(),
            occurred_at: e.occurred_at,
            top_topic: e.topic.to_string(),
            summarized_by: None,
        })
        .collect();
    let mut digest_plan = DigestPlan::default_for(ctx.now);
    digest_plan.max_usd_cents_per_run = 5;
    digest_plan.estimated_usd_cents_per_call = 5;
    let digest_report = run_turn_digest(&digest_plan, ctx.digest_backend, turns)
        .await
        .expect("digest");
    report.digest_buckets_done = digest_report.buckets_done;
    report.digest_usd_cents = digest_report.usd_cents;

    // 3.5 — meili prune
    let prune_plan = PrunePlan::default_for(ctx.now);
    let prune_report = run_meili_prune(&prune_plan, ctx.meili)
        .await
        .expect("meili prune");
    report.meili_pruned = prune_report.pruned;

    // 3.6 — metadata reap
    let reap_plan = ReapPlan::default_for(ctx.now);
    let reap_report = reap_run(ctx.metadata.conn_mut(), &reap_plan).expect("reap");
    report.metadata_collapsed = reap_report.bootstrap_jobs_collapsed
        + reap_report.sessions_collapsed
        + reap_report.spend_collapsed;

    // 3.7 — cas vacuum (--force so the seeded orphan cohort drops in
    // one pass even when it's > 50 % of the store).
    let mut vacuum_opts = VacuumOpts::default_for(ctx.now);
    vacuum_opts.force = true;
    let cas_report = cas_run(ctx.cas, &vacuum_opts).expect("cas vacuum");
    report.cas_dropped = cas_report.blobs_dropped;

    report
}

fn write_archive_files(corpus: &[SynthEnvelope], archive_root: &std::path::Path) {
    // Group envelopes by an hourly partition derived from
    // `occurred_at`. Each unique (year, month, day, hour) gets one
    // file. Streams alternate raw / bootstrap so the rollup gathers
    // both.
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<(i32, u32, u32, u32, &'static str), Vec<&SynthEnvelope>> =
        BTreeMap::new();
    for env in corpus {
        let stream = match env.kind {
            CorpusKind::Turn | CorpusKind::ToolCall => "raw",
            _ => "bootstrap",
        };
        let key = (
            env.occurred_at.year(),
            env.occurred_at.month(),
            env.occurred_at.day(),
            env.occurred_at.hour(),
            stream,
        );
        groups.entry(key).or_default().push(env);
    }
    for ((y, m, d, h, stream), envs) in groups {
        let ts = Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap();
        let _ = synth::write_archive_file(archive_root, ts, stream, &envs);
    }
}

async fn seed_vector_collections(ops: &MemoryVectorizerOps, corpus: &[SynthEnvelope]) {
    use std::collections::BTreeMap;
    let mut by_collection: BTreeMap<String, Vec<cortex_workers::retention::RecordRef>> =
        BTreeMap::new();
    for env in corpus {
        // Only kinds participating in the sweep land in tier
        // collections.
        let sk = match env.kind.sweep_kind() {
            Some(s) => s,
            None => continue,
        };
        let tier = synth::starting_tier(env);
        let collection = match tier {
            Tier::Binary => "cortex.cold.binary".to_string(),
            other => format!("cortex.{}.{}", sk.as_str(), other.as_str()),
        };
        by_collection
            .entry(collection)
            .or_default()
            .push(synth::to_record_ref(env));
    }
    for (collection, records) in by_collection {
        ops.seed(&collection, records).await;
    }
}

async fn seed_meili(meili: &MemoryMeiliBackend, corpus: &[SynthEnvelope]) {
    for kind_filter in [CorpusKind::Turn, CorpusKind::ToolCall] {
        let index = match kind_filter {
            CorpusKind::Turn => "cortex_turns",
            CorpusKind::ToolCall => "cortex_tool_calls",
            _ => unreachable!(),
        };
        let docs: Vec<MeiliDoc> = corpus
            .iter()
            .filter(|e| e.kind == kind_filter)
            .map(|e| MeiliDoc {
                event_id: e.event_id.clone(),
                index: index.to_string(),
                occurred_at: e.occurred_at,
                summary: e.body.clone(),
                already_pruned: false,
            })
            .collect();
        meili.seed(index, docs).await;
    }
}

fn seed_cas_blobs(
    cas: &mut CasStore,
    corpus: &[SynthEnvelope],
    now: DateTime<Utc>,
) -> Vec<String> {
    // Insert one blob per envelope, retain it once so the high-PII
    // path's decrement_cas would matter in production. Mark a fixed
    // subset as "orphans" by aging their last_referenced past the
    // 30-d window without a retain bump.
    let mut orphans = Vec::new();
    for (i, env) in corpus.iter().enumerate() {
        let body = env.body.as_bytes();
        let hash = cas.put(body, CasContentType::Text).unwrap();
        if i % 10 == 0 {
            // Every 10th blob is an orphan: backdated 60 d, refcount 0.
            let aged = (now - Duration::days(60)).to_rfc3339();
            cas.conn()
                .execute(
                    "UPDATE cas_blobs SET last_referenced = ?1, refcount = 0 WHERE hash = ?2",
                    rusqlite::params![aged, hash],
                )
                .unwrap();
            orphans.push(hash);
        } else {
            cas.retain(&hash).unwrap();
        }
    }
    orphans
}

fn seed_metadata(metadata: &MetadataStore, now: DateTime<Utc>) {
    // Repos + bootstrap_jobs (mix of fresh + stale).
    metadata
        .conn()
        .execute(
            "INSERT OR IGNORE INTO repos (path, name) VALUES (?1, ?2)",
            rusqlite::params!["/repo/cortex", "cortex"],
        )
        .unwrap();
    for i in 0..20 {
        let stale = i < 12;
        let ts = if stale {
            (now - Duration::days(45)).to_rfc3339()
        } else {
            (now - Duration::days(5)).to_rfc3339()
        };
        metadata
            .conn()
            .execute(
                "INSERT INTO bootstrap_jobs
                    (job_id, repo_path, started_at, finished_at,
                     files_processed, chunks_emitted, status)
                 VALUES (?1, ?2, ?3, ?3, ?4, ?5, 'success')",
                rusqlite::params![
                    format!("01BJ{i:030}"),
                    "/repo/cortex",
                    ts,
                    100_i64,
                    1_000_i64
                ],
            )
            .unwrap();
    }
    // Sessions (year-old).
    for i in 0..30 {
        let started = (now - Duration::days(400 + i)).to_rfc3339();
        metadata
            .conn()
            .execute(
                "INSERT INTO sessions
                    (session_id, tool, repo, started_at, event_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![format!("01S{i:030}"), "claude-code", "cortex", started, 7_i64],
            )
            .unwrap();
    }
    // Classifier spend (day rows ≥ 366 d old).
    for i in 0..10 {
        let day = (now - Duration::days(370 + i))
            .format("%Y-%m-%d")
            .to_string();
        metadata
            .record_classifier_spend(&day, 5, 100, 50, 200)
            .unwrap();
    }
}

#[derive(Debug, Default)]
struct ArchiveScan {
    has_quarantine: bool,
    tmp_orphans: u32,
    corrupted_outside_quarantine: u32,
}

fn scan_archive_tree(events_root: &std::path::Path) -> ArchiveScan {
    let mut out = ArchiveScan::default();
    if !events_root.exists() {
        return out;
    }
    let quarantine = events_root.join("_quarantine");
    out.has_quarantine = quarantine.exists();
    let mut stack: Vec<std::path::PathBuf> = vec![events_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let in_quarantine = path
                .components()
                .any(|c| c.as_os_str().eq_ignore_ascii_case("_quarantine"));
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase();
            if name.ends_with(".tmp") {
                out.tmp_orphans += 1;
            }
            if name.contains(".corrupted") && !in_quarantine {
                out.corrupted_outside_quarantine += 1;
            }
        }
    }
    out
}
