//! Side-effect-free SQLite inspection while cooperating writers are excluded.
use crate::{files, LocalError, Result};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags};
use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

pub(crate) struct ReadonlyDatabase {
    connection: Connection,
    _lock: File,
}

impl ReadonlyDatabase {
    pub(crate) fn open(
        path: &Path,
        lock_path: &Path,
        pending: Option<&Path>,
        timeout: Duration,
        expected_version: u32,
    ) -> Result<Self> {
        files::ensure_no_symlinks(lock_path)?;
        let lock = File::open(lock_path)?;
        let started = Instant::now();
        loop {
            match FileExt::try_lock_shared(&lock) {
                Ok(()) => break,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    let remaining = timeout.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::WouldBlock,
                            "read-only inspection lock timed out",
                        )
                        .into());
                    }
                    std::thread::sleep(remaining.min(Duration::from_millis(10)));
                }
                Err(error) => return Err(error.into()),
            }
        }
        if let Some(pending) = pending {
            files::ensure_no_symlinks(pending)?;
            if pending.exists() {
                return Err(LocalError::Invalid(
                    "dry-run requires a recovered destination index".into(),
                ));
            }
        }
        files::ensure_no_symlinks(path)?;
        if !path.is_file() {
            return Err(LocalError::Invalid("read-only database is missing".into()));
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            let sidecar = Path::new(&sidecar);
            files::ensure_no_symlinks(sidecar)?;
            if suffix != "-shm" && sidecar.exists() && sidecar.metadata()?.len() != 0 {
                return Err(LocalError::Invalid(
                    "dry-run requires a checkpointed destination database".into(),
                ));
            }
        }
        // immutable=1 avoids SQLite creating WAL/SHM files even for a read-only
        // connection. It is safe only under this lock and with no pending journal.
        let uri = immutable_uri(path)?;
        let connection = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != i64::from(expected_version) {
            return Err(LocalError::Invalid(format!(
                "unsupported read-only database schema {version}; expected {expected_version}"
            )));
        }
        Ok(Self {
            connection,
            _lock: lock,
        })
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }
}

fn immutable_uri(path: &Path) -> Result<String> {
    let canonical = std::fs::canonicalize(path)?;
    let path = canonical
        .to_str()
        .ok_or_else(|| LocalError::Invalid("SQLite inspection path is not UTF-8".into()))?;
    #[cfg(windows)]
    let normalized = if let Some(unc) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("//{}", unc.replace('\\', "/"))
    } else {
        path.strip_prefix("\\\\?\\")
            .unwrap_or(path)
            .replace('\\', "/")
    };
    #[cfg(not(windows))]
    let normalized = path.to_owned();
    let mut uri = if normalized.starts_with('/') {
        "file://"
    } else {
        "file:///"
    }
    .to_owned();
    for byte in normalized.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(uri, "%{byte:02X}").map_err(|error| LocalError::Invalid(error.to_string()))?;
        }
    }
    uri.push_str("?immutable=1");
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaptureRequest, LocalStore};
    use std::collections::BTreeMap;

    fn snapshot(path: &Path) -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(path)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn immutable_inspection_preserves_files_and_excludes_writers() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("space #percent% \u{e9}");
        let store = LocalStore::open_with_lock_timeout(&root, Duration::from_millis(20)).unwrap();
        let before = snapshot(store.root());
        {
            let readonly = store.readonly_index().unwrap();
            let count: i64 = readonly
                .connection()
                .query_row("SELECT count(*) FROM revisions", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 0);
            assert!(store
                .capture(CaptureRequest {
                    repository: "Cortex".into(),
                    ..Default::default()
                })
                .is_err());
            assert_eq!(snapshot(store.root()), before);
        }
        assert_eq!(snapshot(store.root()), before);
        assert!(store
            .capture(CaptureRequest {
                repository: "Cortex".into(),
                ..Default::default()
            })
            .is_ok());
    }
}
