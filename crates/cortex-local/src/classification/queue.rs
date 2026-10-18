//! Explicit, bounded classification work. The queue lock never holds a store lock.
use super::{
    prepare, ClassificationAdvice, FailureKind, PreparedClassification, Provider, ProviderFailure,
    TokenUsage,
};
use crate::{
    files, Authority, CaptureRequest, LocalError, LocalStore, RecordKind, Relation, Result, Source,
    TypedLink,
};
use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

const MAX_JOB_BYTES: u64 = 64 * 1024;
const MAX_SCAN: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Completed,
    Failed,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub schema_version: u32,
    pub key: String,
    pub prepared: PreparedClassification,
    pub status: JobStatus,
    pub attempts: u8,
    pub created_at: String,
    pub next_attempt_at: i64,
    pub last_failure: Option<ProviderFailure>,
    pub known_input_tokens: u64,
    pub known_output_tokens: u64,
    pub unknown_usage_attempts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobList {
    pub jobs: Vec<Job>,
    pub malformed: usize,
    pub scanned: usize,
    pub incomplete: bool,
}

#[derive(Debug, Clone)]
pub struct DrainOptions {
    pub repository: String,
    pub max_jobs: usize,
    pub deadline: Duration,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DrainReport {
    pub attempted: usize,
    pub completed: usize,
    pub cached: usize,
    pub pending: usize,
    pub failed: usize,
    pub stale: usize,
    pub malformed: usize,
    pub incomplete: bool,
    pub deadline_exhausted: bool,
}

pub struct ClassificationQueue {
    store: LocalStore,
}

impl ClassificationQueue {
    #[must_use]
    pub fn new(store: LocalStore) -> Self {
        Self { store }
    }

    fn directory(&self, repository: &str) -> PathBuf {
        self.store
            .root()
            .join("classification")
            .join("jobs")
            .join(files::hash(repository.as_bytes()))
    }

    fn lock(&self, repository: &str) -> Result<File> {
        files::validate_scope(repository)?;
        let directory = self.directory(repository);
        files::ensure_no_symlinks(&directory)?;
        fs::create_dir_all(&directory)?;
        files::ensure_no_symlinks(&directory)?;
        for path in [
            &directory,
            &self.store.root().join("classification/jobs"),
            &self.store.root().join("classification"),
            self.store.root(),
        ] {
            files::sync_directory(path)?;
        }
        let path = self.store.root().join("classification/queue.lock");
        files::ensure_no_symlinks(&path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let start = Instant::now();
        loop {
            match lock.try_lock_exclusive() {
                Ok(()) => return Ok(lock),
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && start.elapsed() < Duration::from_secs(2) =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn path(&self, repository: &str, key: &str) -> Result<PathBuf> {
        if key.len() != 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(LocalError::Invalid("invalid classification key".into()));
        }
        Ok(self.directory(repository).join(format!("{key}.json")))
    }

    fn validate(job: &Job) -> Result<()> {
        if job.schema_version != 1
            || job.attempts > 5
            || job.key != job.prepared.cache_key()?
            || chrono::DateTime::parse_from_rfc3339(&job.created_at).is_err()
        {
            return Err(LocalError::Invalid("invalid classification job".into()));
        }
        Ok(())
    }

    fn read(&self, repository: &str, key: &str) -> Result<Option<Job>> {
        let path = self.path(repository, key)?;
        files::ensure_no_symlinks(&path)?;
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_JOB_BYTES {
            return Err(LocalError::Invalid(
                "classification job exceeds limit or is not regular".into(),
            ));
        }
        let mut bytes = Vec::new();
        file.take(MAX_JOB_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_JOB_BYTES {
            return Err(LocalError::Invalid(
                "classification job exceeds limit".into(),
            ));
        }
        let job: Job = serde_json::from_slice(&bytes)?;
        Self::validate(&job)?;
        if job.key != key || job.prepared.repository != repository {
            return Err(LocalError::Invalid(
                "classification filename mismatch".into(),
            ));
        }
        Ok(Some(job))
    }

    fn save(&self, job: &Job) -> Result<()> {
        Self::validate(job)?;
        let bytes = files::canonical(job)?;
        if bytes.len() as u64 > MAX_JOB_BYTES {
            return Err(LocalError::Invalid(
                "classification job exceeds limit".into(),
            ));
        }
        let path = self.path(&job.prepared.repository, &job.key)?;
        files::ensure_no_symlinks(&path)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(self.directory(&job.prepared.repository))?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|e| e.error)?;
        files::sync_directory(&self.directory(&job.prepared.repository))
    }

    pub fn enqueue(&self, repository: &str, id: &str, model: &str) -> Result<Job> {
        let _lock = self.lock(repository)?;
        let record = self
            .store
            .get(repository, id)?
            .ok_or_else(|| LocalError::NotFound("classification subject".into()))?;
        let prepared = prepare(&record, model)?;
        let key = prepared.cache_key()?;
        if let Some(job) = self.read(repository, &key)? {
            return Ok(job);
        }
        let capacity = self.scan(repository, MAX_SCAN)?;
        if capacity.incomplete || capacity.scanned >= MAX_SCAN {
            return Err(LocalError::Invalid(
                "classification repository queue capacity is 10000 entries".into(),
            ));
        }
        let mut job = Job {
            schema_version: 1,
            key,
            prepared,
            status: JobStatus::Pending,
            attempts: 0,
            created_at: Utc::now().to_rfc3339(),
            next_attempt_at: 0,
            last_failure: None,
            known_input_tokens: 0,
            known_output_tokens: 0,
            unknown_usage_attempts: 0,
        };
        if let Some(usage) = self.cached(&job)? {
            Self::recover_usage(&mut job, &usage);
            job.status = JobStatus::Completed;
        }
        self.save(&job)?;
        Ok(job)
    }

    pub fn get(&self, repository: &str, key: &str) -> Result<Option<Job>> {
        files::validate_scope(repository)?;
        let _lock = self.lock(repository)?;
        Ok(self
            .read(repository, key)?
            .filter(|job| job.prepared.repository == repository))
    }

    fn scan(&self, repository: &str, limit: usize) -> Result<JobList> {
        files::validate_scope(repository)?;
        if !(1..=MAX_SCAN).contains(&limit) {
            return Err(LocalError::Invalid(
                "job list limit must be 1..10000".into(),
            ));
        }
        let mut report = JobList {
            jobs: Vec::new(),
            malformed: 0,
            scanned: 0,
            incomplete: false,
        };
        for (index, entry) in fs::read_dir(self.directory(repository))?.enumerate() {
            if index == MAX_SCAN {
                report.incomplete = true;
                break;
            }
            report.scanned += 1;
            let entry = entry?;
            let name = entry.file_name();
            let Some(key) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
                report.malformed += 1;
                continue;
            };
            match self.read(repository, key) {
                Ok(Some(job)) if job.prepared.repository == repository => report.jobs.push(job),
                Ok(_) => (),
                Err(_) => report.malformed += 1,
            }
        }
        report.jobs.sort_by(|a, b| {
            (a.next_attempt_at, &a.created_at, &a.key).cmp(&(
                b.next_attempt_at,
                &b.created_at,
                &b.key,
            ))
        });
        if report.jobs.len() > limit {
            report.incomplete = true;
            report.jobs.truncate(limit);
        }
        Ok(report)
    }

    pub fn list(&self, repository: &str, limit: usize) -> Result<JobList> {
        let _lock = self.lock(repository)?;
        self.scan(repository, limit)
    }

    pub fn retry(&self, repository: &str, key: &str) -> Result<Job> {
        files::validate_scope(repository)?;
        let _lock = self.lock(repository)?;
        let mut job = self
            .read(repository, key)?
            .filter(|job| job.prepared.repository == repository)
            .ok_or_else(|| LocalError::NotFound("classification job".into()))?;
        if job.status != JobStatus::Failed {
            return Err(LocalError::Invalid(
                "only failed jobs can be retried".into(),
            ));
        }
        job.status = JobStatus::Pending;
        job.attempts = 0;
        job.next_attempt_at = 0;
        job.last_failure = None;
        self.save(&job)?;
        Ok(job)
    }

    fn result_id(job: &Job) -> String {
        format!("classification:{}", job.key)
    }

    fn current_matches(&self, job: &Job) -> Result<bool> {
        Ok(self
            .store
            .get(&job.prepared.repository, &job.prepared.record_id)?
            .is_some_and(|record| {
                record.revision == job.prepared.revision
                    && record.content_sha256 == job.prepared.content_sha256
            }))
    }

    fn result_status(&self, job: &Job) -> Result<JobStatus> {
        Ok(if self.current_matches(job)? {
            JobStatus::Completed
        } else {
            JobStatus::Stale
        })
    }

    fn recover_usage(job: &mut Job, usage: &TokenUsage) {
        if job.unknown_usage_attempts > 0 || job.attempts == 0 {
            job.unknown_usage_attempts = job.unknown_usage_attempts.saturating_sub(1);
            job.known_input_tokens = job.known_input_tokens.saturating_add(usage.input_tokens);
            job.known_output_tokens = job.known_output_tokens.saturating_add(usage.output_tokens);
        }
    }

    fn cached(&self, job: &Job) -> Result<Option<TokenUsage>> {
        let Some(record) = self
            .store
            .get(&job.prepared.repository, &Self::result_id(job))?
        else {
            return Ok(None);
        };
        let valid = record.kind == RecordKind::Event
            && record.authority == Authority::Observation
            && record.source.kind == "jev_classification"
            && record.metadata.get("prepared") == Some(&serde_json::to_value(&job.prepared)?);
        let advice = record
            .metadata
            .get("advice")
            .cloned()
            .and_then(|value| serde_json::from_value::<ClassificationAdvice>(value).ok());
        if !valid
            || advice.as_ref().map_or(true, |advice| {
                advice.validate(&job.prepared.requested_model).is_err()
            })
        {
            return Err(LocalError::Conflict(
                "invalid canonical classification result".into(),
            ));
        }
        Ok(advice.map(|advice| advice.usage))
    }

    fn complete(&self, job: &Job, advice: ClassificationAdvice) -> Result<()> {
        advice.validate(&job.prepared.requested_model)?;
        self.store.capture(CaptureRequest {
            id: Some(Self::result_id(job)), repository: job.prepared.repository.clone(), kind: RecordKind::Event, authority: Authority::Observation,
            title: "Advisory Jev classification".into(), body: "Classification advice; source authority remains unchanged.".into(),
            source: Source { kind: "jev_classification".into(), locator: format!("cortex:classification:{}",job.key), source_id: Some(job.key.clone()), captured_at: Some(job.created_at.clone()), source_hash: Some(job.prepared.request_sha256.clone()), payload: json!({}) },
            links: vec![TypedLink { target_id: job.prepared.record_id.clone(), relation: Relation::DerivedFrom }],
            metadata: json!({"schema_version":1,"cache_key":job.key,"prepared":job.prepared,"advice":advice}),
        })?;
        Ok(())
    }

    pub async fn drain(&self, provider: &Provider, options: DrainOptions) -> Result<DrainReport> {
        if !(1..=100).contains(&options.max_jobs)
            || options.deadline < Duration::from_secs(1)
            || options.deadline > Duration::from_secs(60)
        {
            return Err(LocalError::Invalid(
                "drain requires 1..100 jobs and 1..60 seconds".into(),
            ));
        }
        let start = Instant::now();
        let _lock = self.lock(&options.repository)?;
        let scanned = self.scan(&options.repository, MAX_SCAN)?;
        let mut report = DrainReport {
            malformed: scanned.malformed,
            incomplete: scanned.incomplete,
            ..Default::default()
        };
        let mut handled = 0;
        for mut job in scanned.jobs {
            if job.status == JobStatus::Pending
                && handled < options.max_jobs
                && start.elapsed() < options.deadline
            {
                if let Some(usage) = self.cached(&job)? {
                    Self::recover_usage(&mut job, &usage);
                    job.status = self.result_status(&job)?;
                    report.cached += 1;
                    handled += 1;
                    self.save(&job)?;
                } else if job.next_attempt_at <= Utc::now().timestamp() {
                    handled += 1;
                    let current = self
                        .store
                        .get(&job.prepared.repository, &job.prepared.record_id)?;
                    if current.map_or(true, |record| {
                        record.revision != job.prepared.revision
                            || record.content_sha256 != job.prepared.content_sha256
                    }) {
                        job.status = JobStatus::Stale;
                    } else if job.attempts >= 5 {
                        job.status = JobStatus::Failed;
                    } else if provider.is_configured() {
                        job.attempts += 1;
                        let delay = (5_i64 * (1_i64 << (job.attempts - 1))).min(300);
                        job.next_attempt_at = Utc::now().timestamp() + 60 + delay;
                        job.unknown_usage_attempts = job.unknown_usage_attempts.saturating_add(1);
                        self.save(&job)?;
                        report.attempted += 1;
                        let remaining = options.deadline.saturating_sub(start.elapsed());
                        let result =
                            tokio::time::timeout(remaining, provider.classify(&job.prepared))
                                .await
                                .unwrap_or(Err(ProviderFailure {
                                    kind: FailureKind::Timeout,
                                    retryable: true,
                                    retry_after_secs: None,
                                    usage: None,
                                }));
                        job.next_attempt_at = Utc::now().timestamp() + delay;
                        let usage: Option<&TokenUsage> = match &result {
                            Ok(advice) => Some(&advice.usage),
                            Err(failure) => failure.usage.as_ref(),
                        };
                        if let Some(usage) = usage {
                            job.unknown_usage_attempts =
                                job.unknown_usage_attempts.saturating_sub(1);
                            job.known_input_tokens =
                                job.known_input_tokens.saturating_add(usage.input_tokens);
                            job.known_output_tokens =
                                job.known_output_tokens.saturating_add(usage.output_tokens);
                        }
                        match result {
                            Ok(advice) => {
                                self.complete(&job, advice)?;
                                job.status = self.result_status(&job)?;
                                job.last_failure = None;
                            }
                            Err(failure) => {
                                if !failure.retryable || job.attempts >= 5 {
                                    job.status = JobStatus::Failed;
                                }
                                if let Some(seconds) = failure.retry_after_secs {
                                    job.next_attempt_at = Utc::now().timestamp()
                                        + seconds.clamp(delay as u64, 300) as i64;
                                }
                                job.last_failure = Some(failure);
                            }
                        }
                    }
                    self.save(&job)?;
                }
            }
            match job.status {
                JobStatus::Pending => report.pending += 1,
                JobStatus::Completed => report.completed += 1,
                JobStatus::Failed => report.failed += 1,
                JobStatus::Stale => report.stale += 1,
            }
        }
        report.deadline_exhausted = start.elapsed() >= options.deadline;
        Ok(report)
    }
}
