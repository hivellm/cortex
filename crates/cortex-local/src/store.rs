//! One local engine, usable without HTTP, Docker or background processes.
use crate::{files, graph, index, model::*};
use fs2::FileExt;
use rusqlite::{Connection, TransactionBehavior};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Canonical store root (normally `<CORTEX_HOME>/local`). Connections are short
/// lived; an OS lock serializes recovery and writes across threads and processes.
#[derive(Debug, Clone)]
pub struct LocalStore {
    root: PathBuf,
    lock_timeout: Duration,
    hook_only: bool,
    inspection_only: bool,
}

impl LocalStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_lock_timeout(root, Duration::from_secs(30))
    }

    /// Lifecycle hooks can select a short lock budget instead of waiting behind
    /// a historical import. A zero budget makes one immediate lock attempt.
    pub fn open_with_lock_timeout(root: impl AsRef<Path>, lock_timeout: Duration) -> Result<Self> {
        let root = if root.as_ref().is_absolute() {
            root.as_ref().to_path_buf()
        } else {
            std::env::current_dir()?.join(root)
        };
        let store = Self {
            root: files::prepare_root(&root)?,
            lock_timeout,
            hook_only: false,
            inspection_only: false,
        };
        store.with_connection(|_| Ok(()))?;
        Ok(store)
    }

    /// Fail closed when initialization or recovery is needed. This policy stays
    /// attached to the handle, including when a later write leaves pending work.
    pub fn open_for_hook(root: impl AsRef<Path>, timeout: Duration) -> Result<Self> {
        let root = files::existing_root(root.as_ref())?;
        let store = Self {
            root,
            lock_timeout: timeout.min(Duration::from_millis(200)),
            hook_only: true,
            inspection_only: false,
        };
        store.with_connection(|_| Ok(()))?;
        Ok(store)
    }

    /// Construct a strictly read-only handle for import dry-runs. Normal store
    /// operations are unavailable on this handle; import uses immutable SQLite.
    pub fn inspect(root: impl AsRef<Path>) -> Result<Self> {
        let store = Self {
            root: files::existing_root(root.as_ref())?,
            lock_timeout: Duration::from_secs(2),
            hook_only: true,
            inspection_only: true,
        };
        index::validate_schema(store.readonly_index()?.connection())?;
        Ok(store)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn readonly_index(&self) -> Result<crate::readonly::ReadonlyDatabase> {
        crate::readonly::ReadonlyDatabase::open(
            &self.root.join("index.sqlite3"),
            &self.root.join("store.lock"),
            Some(&self.root.join("pending")),
            self.lock_timeout,
        )
    }

    fn lock(&self) -> Result<File> {
        files::ensure_no_symlinks(&self.root)?;
        let path = self.root.join("store.lock");
        files::ensure_no_symlinks(&path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(!self.hook_only)
            .truncate(false)
            .open(path)?;
        let started = Instant::now();
        loop {
            match FileExt::try_lock_exclusive(&lock) {
                Ok(()) => return Ok(lock),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    let Some(remaining) = self.lock_timeout.checked_sub(started.elapsed()) else {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::WouldBlock,
                            "local store lock timed out",
                        )
                        .into());
                    };
                    if remaining.is_zero() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::WouldBlock,
                            "local store lock timed out",
                        )
                        .into());
                    }
                    std::thread::sleep(remaining.min(Duration::from_millis(10)));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn with_connection<T>(&self, action: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        if self.inspection_only {
            return Err(LocalError::Invalid(
                "inspection handle supports read-only import dry-runs only".into(),
            ));
        }
        let _lock = self.lock()?;
        let missing_index = !self.root.join("index.sqlite3").exists();
        let pending = self.root.join("pending");
        files::ensure_no_symlinks(&pending)?;
        if self.hook_only {
            if missing_index || pending.exists() {
                return Err(LocalError::Invalid(
                    "hook index requires explicit recovery".into(),
                ));
            }
            let mut connection = index::connect_existing(&self.root, self.lock_timeout)?;
            return action(&mut connection);
        }
        if missing_index && !pending.exists() {
            self.begin_write()?;
        }
        let (mut connection, uninitialized) =
            index::connect(&self.root, self.lock_timeout, || {
                if !pending.exists() {
                    self.begin_write()?;
                }
                Ok(())
            })?;
        if missing_index || uninitialized || pending.exists() {
            index::rebuild(&self.root, &mut connection)?;
            self.finish_write()?;
        }
        action(&mut connection)
    }

    fn begin_write(&self) -> Result<()> {
        let path = self.root.join("pending");
        files::ensure_no_symlinks(&path)?;
        let mut marker = OpenOptions::new().write(true).create_new(true).open(path)?;
        marker.write_all(b"canonical publication may precede index commit\n")?;
        marker.sync_all()?;
        files::sync_directory(&self.root)
    }

    fn finish_write(&self) -> Result<()> {
        match fs::remove_file(self.root.join("pending")) {
            Ok(()) => files::sync_directory(&self.root),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn capture(&self, request: CaptureRequest) -> Result<WriteOutcome> {
        if request.kind == RecordKind::Event {
            return self
                .capture_batch(vec![request])?
                .records
                .into_iter()
                .next()
                .ok_or_else(|| LocalError::Invalid("empty event capture outcome".into()));
        }
        self.with_connection(|connection| {
            let candidate = make_record(request)?;
            if let Some(existing) =
                index::get(connection, &candidate.repository, &candidate.id, Some(1))?
            {
                return duplicate(existing, &candidate);
            }
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            self.begin_write()?;
            files::publish_memory(&self.root, &candidate)?;
            index::project(&transaction, &candidate)?;
            transaction.commit()?;
            self.finish_write()?;
            Ok(WriteOutcome {
                record: candidate,
                created: true,
            })
        })
    }

    /// Apply canonical record constraints without touching the store or publishing data.
    pub fn validate_capture(request: &CaptureRequest) -> Result<()> {
        make_record(request.clone()).map(|_| ())
    }

    /// Persist one bounded, immutable segment. A batch is fully validated before
    /// publication; retries are idempotent when callers retain the same IDs.
    pub fn capture_batch(&self, requests: Vec<CaptureRequest>) -> Result<BatchOutcome> {
        if requests.is_empty() || requests.len() > MAX_BATCH_RECORDS {
            return Err(LocalError::Invalid(
                "batch must contain 1..256 events".into(),
            ));
        }
        self.with_connection(|connection| {
            let mut outcome = BatchOutcome::default();
            let mut new_records = Vec::new();
            let mut batch_records: HashMap<(String, String), Record> = HashMap::new();
            let mut bytes = 0;
            for request in requests {
                if request.kind != RecordKind::Event || request.authority != Authority::Observation
                {
                    return Err(LocalError::Invalid(
                        "batch accepts only raw event observations".into(),
                    ));
                }
                let candidate = make_record(request)?;
                let key = (candidate.repository.clone(), candidate.id.clone());
                let existing = match batch_records.get(&key) {
                    Some(existing) => Some(existing.clone()),
                    None => index::get(connection, &candidate.repository, &candidate.id, Some(1))?,
                };
                if let Some(existing) = existing {
                    outcome.records.push(duplicate(existing, &candidate)?);
                    outcome.duplicates += 1;
                } else {
                    bytes += files::canonical(&candidate)?.len() + 1;
                    if bytes > MAX_BATCH_BYTES {
                        return Err(LocalError::Invalid(
                            "event segment exceeds 8 MiB; split batch".into(),
                        ));
                    }
                    batch_records.insert(key, candidate.clone());
                    outcome.records.push(WriteOutcome {
                        record: candidate.clone(),
                        created: true,
                    });
                    new_records.push(candidate);
                    outcome.created += 1;
                }
            }
            if new_records.is_empty() {
                return Ok(outcome);
            }
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            self.begin_write()?;
            files::publish_segment(&self.root, &new_records)?;
            for record in &new_records {
                index::project(&transaction, record)?;
            }
            transaction.commit()?;
            self.finish_write()?;
            Ok(outcome)
        })
    }

    pub fn revise(
        &self,
        repository: &str,
        id: &str,
        expected_revision: u64,
        request: RevisionRequest,
    ) -> Result<WriteOutcome> {
        files::validate_scope(repository)?;
        files::validate_id(id)?;
        self.with_connection(|connection| {
            let mut record = index::get(connection, repository, id, None)?
                .ok_or_else(|| LocalError::NotFound(id.to_owned()))?;
            if record.revision != expected_revision {
                return Err(LocalError::Conflict(format!(
                    "{} expected revision {}, current {}",
                    id, expected_revision, record.revision
                )));
            }
            if record.kind == RecordKind::Event {
                return Err(LocalError::Invalid(
                    "raw events are immutable; capture a derived memory instead".into(),
                ));
            }
            record.revision += 1;
            record.updated_at = chrono::Utc::now().to_rfc3339();
            record.title = request.title;
            record.body = request.body;
            record.authority = request.authority;
            record.links = request.links;
            record.metadata = request.metadata;
            files::seal(&mut record)?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            self.begin_write()?;
            files::publish_memory(&self.root, &record)?;
            index::project(&transaction, &record)?;
            transaction.commit()?;
            self.finish_write()?;
            Ok(WriteOutcome {
                record,
                created: true,
            })
        })
    }

    pub fn get(&self, repository: &str, id: &str) -> Result<Option<Record>> {
        files::validate_scope(repository)?;
        files::validate_id(id)?;
        self.with_connection(|connection| index::get(connection, repository, id, None))
    }

    /// Resolve a bounded group under one store lock, preserving input order and misses.
    pub fn get_many(&self, repository: &str, ids: &[String]) -> Result<Vec<Option<Record>>> {
        files::validate_scope(repository)?;
        if ids.len() > MAX_BATCH_RECORDS {
            return Err(LocalError::Invalid(
                "lookup batch exceeds 256 identities".into(),
            ));
        }
        for id in ids {
            files::validate_id(id)?;
        }
        self.with_connection(|connection| {
            ids.iter()
                .map(|id| index::get(connection, repository, id, None))
                .collect()
        })
    }

    pub fn get_revision(
        &self,
        repository: &str,
        id: &str,
        revision: u64,
    ) -> Result<Option<Record>> {
        files::validate_scope(repository)?;
        files::validate_id(id)?;
        if revision == 0 || revision > i64::MAX as u64 {
            return Err(LocalError::Invalid(
                "revision outside supported range".into(),
            ));
        }
        self.with_connection(|connection| index::get(connection, repository, id, Some(revision)))
    }

    pub fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        self.with_connection(|connection| index::search(connection, query, false))
    }

    pub fn search_memories(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        self.with_connection(|connection| index::search(connection, query, true))
    }

    pub fn recent(
        &self,
        repository: &str,
        limit: usize,
        include_inactive: bool,
    ) -> Result<Vec<Record>> {
        self.with_connection(|connection| {
            index::recent(connection, repository, limit, include_inactive, false)
        })
    }

    /// Filter event history before applying the context budget.
    pub fn recent_memories(
        &self,
        repository: &str,
        limit: usize,
        include_inactive: bool,
    ) -> Result<Vec<Record>> {
        self.with_connection(|connection| {
            index::recent(connection, repository, limit, include_inactive, true)
        })
    }

    pub fn graph(&self, query: GraphQuery) -> Result<GraphResult> {
        self.with_connection(|connection| graph::query(connection, query))
    }

    pub fn status(&self, repository: Option<&str>) -> Result<StoreStatus> {
        self.with_connection(|connection| index::status(connection, repository))
    }

    pub fn reindex(&self) -> Result<ReindexReport> {
        if self.hook_only {
            return Err(LocalError::Invalid(
                "hook handles cannot rebuild indexes".into(),
            ));
        }
        self.with_connection(|connection| index::rebuild(&self.root, connection))
    }

    /// Rebuild off to the side before replacing an index. Original SQLite bytes,
    /// sidecars and pending marker remain available in a durable backup folder.
    pub fn repair(root: impl AsRef<Path>) -> Result<ReindexReport> {
        Self::repair_with_observer(root, || {})
    }

    fn repair_with_observer(
        root: impl AsRef<Path>,
        mut after_move: impl FnMut(),
    ) -> Result<ReindexReport> {
        let root = files::prepare_root(root.as_ref())?;
        let store = Self {
            root,
            lock_timeout: Duration::from_secs(30),
            hook_only: false,
            inspection_only: false,
        };
        let _lock = store.lock()?;
        let backups = store.root.join("index-backups");
        files::ensure_no_symlinks(&backups)?;
        fs::create_dir_all(&backups)?;
        let backup = backups.join(ulid::Ulid::new().to_string());
        fs::create_dir(&backup)?;
        let staging = backup.join("replacement");
        fs::create_dir(&staging)?;
        for directory in [&staging, &backup, &backups, &store.root] {
            files::sync_directory(directory)?;
        }
        let (mut connection, _) = index::connect(&staging, store.lock_timeout, || Ok(()))?;
        let report = index::rebuild(&store.root, &mut connection)?;
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        connection
            .close()
            .map_err(|(_, error)| LocalError::Sql(error))?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(staging.join("index.sqlite3"))?
            .sync_all()?;
        files::sync_directory(&staging)?;

        let names = [
            "index.sqlite3",
            "index.sqlite3-wal",
            "index.sqlite3-shm",
            "index.sqlite3-journal",
            "pending",
        ];
        // Preflight every source before moving any original file.
        for name in names {
            let path = store.root.join(name);
            files::ensure_no_symlinks(&path)?;
            if path.exists() && !path.is_file() {
                return Err(LocalError::Invalid(
                    "index repair source is not a regular file".into(),
                ));
            }
            if path.exists() {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)?
                    .sync_all()?;
            }
        }
        // Complete and flush every forensic copy before any rename. A process
        // crash between main-db and WAL moves must not leave the only WAL copy
        // exposed to SQLite's next automatic recovery.
        let originals = backup.join("originals");
        let displaced = backup.join("moved");
        fs::create_dir(&originals)?;
        fs::create_dir(&displaced)?;
        for name in names {
            let source = store.root.join(name);
            if source.exists() {
                let target = originals.join(name);
                fs::copy(source, &target)?;
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(target)?
                    .sync_all()?;
            }
        }
        for directory in [&originals, &displaced, &backup, &backups, &store.root] {
            files::sync_directory(directory)?;
        }
        let mut moved = Vec::new();
        let mut installed = false;
        let swap: Result<()> = (|| {
            for name in names {
                let original = store.root.join(name);
                if original.exists() {
                    fs::rename(&original, displaced.join(name))?;
                    moved.push(name);
                    after_move();
                }
            }
            files::sync_directory(&displaced)?;
            files::sync_directory(&store.root)?;
            fs::rename(
                staging.join("index.sqlite3"),
                store.root.join("index.sqlite3"),
            )?;
            installed = true;
            files::sync_directory(&staging)?;
            files::sync_directory(&store.root)?;
            Ok(())
        })();
        if let Err(error) = swap {
            if installed {
                fs::rename(
                    store.root.join("index.sqlite3"),
                    staging.join("index.sqlite3"),
                )?;
            }
            for name in moved.into_iter().rev() {
                fs::rename(displaced.join(name), store.root.join(name))?;
            }
            files::sync_directory(&store.root)?;
            files::sync_directory(&displaced)?;
            return Err(error);
        }
        Ok(report)
    }
}

fn make_record(request: CaptureRequest) -> Result<Record> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut record = Record {
        schema_version: 1,
        id: request.id.unwrap_or_else(|| ulid::Ulid::new().to_string()),
        repository: request.repository,
        revision: 1,
        kind: request.kind,
        authority: request.authority,
        title: request.title,
        body: request.body,
        source: request.source,
        links: request.links,
        metadata: request.metadata,
        created_at: now.clone(),
        updated_at: now,
        content_sha256: String::new(),
    };
    files::seal(&mut record)?;
    Ok(record)
}

fn duplicate(existing: Record, candidate: &Record) -> Result<WriteOutcome> {
    let mut normalized = candidate.clone();
    normalized.created_at.clone_from(&existing.created_at);
    normalized.updated_at.clone_from(&existing.updated_at);
    files::seal(&mut normalized)?;
    if existing != normalized {
        return Err(LocalError::Conflict(format!(
            "identity {} already contains different input",
            candidate.id
        )));
    }
    Ok(WriteOutcome {
        record: existing,
        created: false,
    })
}

#[cfg(test)]
mod repair_crash_tests {
    use super::*;

    #[test]
    fn interrupted_first_move_keeps_every_original_byte_backed_up() {
        let root = tempfile::tempdir().unwrap();
        LocalStore::open(root.path()).unwrap();
        let names = [
            "index.sqlite3",
            "index.sqlite3-wal",
            "index.sqlite3-shm",
            "index.sqlite3-journal",
            "pending",
        ];
        for name in names {
            fs::write(root.path().join(name), format!("forensic original {name}")).unwrap();
        }
        let crash = std::panic::catch_unwind(|| {
            let _ = LocalStore::repair_with_observer(root.path(), || {
                panic!("simulated crash after main index move")
            });
        });
        assert!(crash.is_err());
        assert!(!root.path().join("index.sqlite3").exists());
        assert!(root.path().join("index.sqlite3-wal").exists());
        let backup = fs::read_dir(root.path().join("index-backups"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        for name in names {
            assert_eq!(
                fs::read(backup.join("originals").join(name)).unwrap(),
                format!("forensic original {name}").as_bytes()
            );
        }
        // A subsequent explicit repair can recover; the prior forensic bundle
        // remains byte-identical even after orphan sidecars are moved away.
        LocalStore::repair(root.path()).unwrap();
        for name in names {
            assert_eq!(
                fs::read(backup.join("originals").join(name)).unwrap(),
                format!("forensic original {name}").as_bytes()
            );
        }
        assert!(LocalStore::open_for_hook(root.path(), Duration::ZERO).is_ok());
    }
}
