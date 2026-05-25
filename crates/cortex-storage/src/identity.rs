//! ADR-012 — `EventIdentity` cross-backend join key + SQLite
//! `IdentityIndex`. Closes the per-call cross-backend lookup cost
//! the doctor + forget paths absorbed pre-phase13d.
//!
//! ## Why
//!
//! `forget`, dedup, doctor, and retention all need to answer "where
//! does event X live across the five backends?". Pre-ADR-012 each
//! consumer fanned out per-backend lookups — `doctor consistency`
//! over 100k events took minutes; a missed backend silently
//! orphaned a row (the bug that surfaced when Synap was added as a
//! fifth backend in phase11i and `admin_forget` was not updated).
//!
//! ADR-012 collapses every cross-backend op onto an indexed lookup
//! against the new `event_identity` table. Every projection path
//! writes its native id back via [`IdentityIndex::upsert_identity`]
//! immediately after the per-backend insert lands; consumers walk
//! the table once and `exists(backend, native_id)` per row.
//!
//! The table sits next to `sessions` / `retention_sweeps` /
//! `cron_jobs` in `metadata.sqlite`. Synap is the source-of-truth
//! event stream so its id (the row's `event_id`) does not need a
//! column.

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

/// ADR-012 — typed cross-backend join row. One row per `event_id`
/// the ingestion pipeline has ever projected. Backend ids are
/// `Option<String>` because a projection may legitimately skip a
/// backend (e.g. `LawViolation` envelopes do not flow into the
/// embedder, so `vec_id` stays `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventIdentity {
    /// Canonical envelope id (ULID, spec-04).
    pub event_id: String,
    /// Nexus `node_id` for the matching node; `None` until the
    /// graph mapper projects this envelope.
    pub nexus_id: Option<String>,
    /// Vectorizer `vector_id` for the matching embedding; `None`
    /// until the embedder projects this envelope.
    pub vec_id: Option<String>,
    /// Meili `document_id`; `None` until the fulltext indexer
    /// projects this envelope.
    pub meili_id: Option<String>,
    /// Parquet partition path the envelope lives in (e.g.
    /// `events/year=2026/month=05/day=24/hour=10/raw-00000.parquet`).
    /// `None` until the archive writer projects this envelope.
    pub archive_partition: Option<String>,
}

/// ADR-012 — Backend enum used as the dispatch key for
/// [`IdentityIndex::upsert_identity`] /
/// [`IdentityIndex::lookup_by_native`]. Synap is intentionally
/// absent: Synap's id IS the envelope's `event_id`, so the join
/// is trivial and needs no separate column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backend {
    /// Nexus graph store. Native id is `node_id`.
    Nexus,
    /// Vectorizer embedding store. Native id is `vector_id`.
    Vectorizer,
    /// Meilisearch full-text index. Native id is `document_id`.
    Meili,
    /// Parquet archive. Native id is the partition path.
    Archive,
}

impl Backend {
    /// Stable short label used in tracing + tests. Order matches
    /// the enum variants so iterating over a known-good list keeps
    /// the doctor's report ordering deterministic.
    pub const fn as_str(self) -> &'static str {
        match self {
            Backend::Nexus => "nexus",
            Backend::Vectorizer => "vectorizer",
            Backend::Meili => "meili",
            Backend::Archive => "archive",
        }
    }

    /// All four variants in deterministic order. Lets callers walk
    /// the backend set without enumerating variants inline at every
    /// site.
    pub const fn all() -> [Backend; 4] {
        [
            Backend::Nexus,
            Backend::Vectorizer,
            Backend::Meili,
            Backend::Archive,
        ]
    }
}

/// ADR-012 — errors raised by [`IdentityIndex`] operations. Wraps
/// every rusqlite error so callers (the projection paths) do not
/// have to depend on `rusqlite::Error` directly.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// Underlying SQLite failure.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Caller passed an empty `event_id` or `native_id` — both must
    /// be non-empty strings for the indexed lookups to be useful.
    #[error("empty id: {field}")]
    EmptyId {
        /// `event_id` / `native_id` — the field that was empty.
        field: &'static str,
    },
}

/// ADR-012 — trait that consumers depend on. Implemented by
/// [`SqliteIdentityIndex`] for production and by the in-memory
/// `MemoryIdentityIndex` (test seam in this crate's tests/) for
/// unit tests.
///
/// The trait deliberately does NOT require `Send + Sync` because
/// the production impl wraps `&rusqlite::Connection`, which is
/// `!Sync`. The wider Cortex codebase serialises every
/// `MetadataStore` access behind an outer `Arc<Mutex<…>>`, so
/// `IdentityIndex` impls only need to be valid for the duration
/// of a single lock; cross-thread sharing happens through the
/// `Arc<Mutex<…>>` wrapper, not through the trait object.
pub trait IdentityIndex {
    /// Insert or update the native-id slot for `event_id` on
    /// `backend`. Empty strings are rejected so the unique partial
    /// indexes never see a meaningless value. The other columns
    /// stay at whatever they were before — write-only-per-backend
    /// semantics are intentional so the projection paths can write
    /// in any order without racing each other.
    fn upsert_identity(
        &self,
        event_id: &str,
        backend: Backend,
        native_id: &str,
    ) -> Result<(), IdentityError>;

    /// Fetch the full identity row for `event_id`. `None` when no
    /// projection has stamped a row yet.
    fn lookup(&self, event_id: &str) -> Result<Option<EventIdentity>, IdentityError>;

    /// Reverse lookup — find the `EventIdentity` whose `backend`
    /// column matches `native_id`. Returns the FULL row so callers
    /// can branch on the other columns. The partial UNIQUE index
    /// on the secondary column guarantees at most one match.
    fn lookup_by_native(
        &self,
        backend: Backend,
        native_id: &str,
    ) -> Result<Option<EventIdentity>, IdentityError>;

    /// Drop the row for `event_id`. Called by `admin_forget`
    /// after every per-backend delete completes. Idempotent — a
    /// delete of a non-existent row returns `Ok(())`.
    fn delete(&self, event_id: &str) -> Result<(), IdentityError>;
}

/// Production SQLite-backed [`IdentityIndex`]. Wraps a borrowed
/// connection so the caller controls the transaction boundary.
/// The projection paths and `admin_forget` both share a single
/// `MetadataStore` connection guarded by its outer `Mutex`, so the
/// wrapping is intentional — owning the connection here would
/// double-lock under the workspace's existing serialisation
/// pattern.
pub struct SqliteIdentityIndex<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteIdentityIndex<'a> {
    /// Wrap `conn`. The connection MUST have been migrated via
    /// [`apply_phase13d_schema`] (or via the bundled `MetadataStore`
    /// open path which calls every `apply_*_schema` helper at
    /// startup); this constructor does not run the migration so the
    /// caller can keep migrations centralised.
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

fn validate_id(field: &'static str, value: &str) -> Result<(), IdentityError> {
    if value.trim().is_empty() {
        return Err(IdentityError::EmptyId { field });
    }
    Ok(())
}

impl<'a> IdentityIndex for SqliteIdentityIndex<'a> {
    fn upsert_identity(
        &self,
        event_id: &str,
        backend: Backend,
        native_id: &str,
    ) -> Result<(), IdentityError> {
        validate_id("event_id", event_id)?;
        validate_id("native_id", native_id)?;

        // Column name is derived from `backend` so the per-backend
        // write-back lands in the right column without a runtime
        // dispatch. The `ON CONFLICT(event_id) DO UPDATE` keeps the
        // sibling columns untouched — projections write in any order
        // and the row accretes the four ids over time.
        let sql = match backend {
            Backend::Nexus => {
                "INSERT INTO event_identity (event_id, nexus_id) VALUES (?1, ?2)
                 ON CONFLICT(event_id) DO UPDATE SET nexus_id = excluded.nexus_id"
            }
            Backend::Vectorizer => {
                "INSERT INTO event_identity (event_id, vec_id) VALUES (?1, ?2)
                 ON CONFLICT(event_id) DO UPDATE SET vec_id = excluded.vec_id"
            }
            Backend::Meili => {
                "INSERT INTO event_identity (event_id, meili_id) VALUES (?1, ?2)
                 ON CONFLICT(event_id) DO UPDATE SET meili_id = excluded.meili_id"
            }
            Backend::Archive => {
                "INSERT INTO event_identity (event_id, archive_partition) VALUES (?1, ?2)
                 ON CONFLICT(event_id) DO UPDATE SET archive_partition = excluded.archive_partition"
            }
        };
        self.conn.execute(sql, params![event_id, native_id])?;
        Ok(())
    }

    fn lookup(&self, event_id: &str) -> Result<Option<EventIdentity>, IdentityError> {
        validate_id("event_id", event_id)?;
        let row = self
            .conn
            .query_row(
                "SELECT event_id, nexus_id, vec_id, meili_id, archive_partition
                 FROM event_identity WHERE event_id = ?1",
                params![event_id],
                |r| {
                    Ok(EventIdentity {
                        event_id: r.get(0)?,
                        nexus_id: r.get(1)?,
                        vec_id: r.get(2)?,
                        meili_id: r.get(3)?,
                        archive_partition: r.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    fn lookup_by_native(
        &self,
        backend: Backend,
        native_id: &str,
    ) -> Result<Option<EventIdentity>, IdentityError> {
        validate_id("native_id", native_id)?;
        let column = match backend {
            Backend::Nexus => "nexus_id",
            Backend::Vectorizer => "vec_id",
            Backend::Meili => "meili_id",
            Backend::Archive => "archive_partition",
        };
        let sql = format!(
            "SELECT event_id, nexus_id, vec_id, meili_id, archive_partition
             FROM event_identity WHERE {column} = ?1"
        );
        let row = self
            .conn
            .query_row(&sql, params![native_id], |r| {
                Ok(EventIdentity {
                    event_id: r.get(0)?,
                    nexus_id: r.get(1)?,
                    vec_id: r.get(2)?,
                    meili_id: r.get(3)?,
                    archive_partition: r.get(4)?,
                })
            })
            .optional()?;
        Ok(row)
    }

    fn delete(&self, event_id: &str) -> Result<(), IdentityError> {
        validate_id("event_id", event_id)?;
        self.conn.execute(
            "DELETE FROM event_identity WHERE event_id = ?1",
            params![event_id],
        )?;
        Ok(())
    }
}

/// ADR-012 — apply the phase13d schema to `conn`. Idempotent
/// (`CREATE TABLE IF NOT EXISTS`). Mirrors the per-phase migration
/// pattern in `metadata.rs` (`apply_phase13b_schema`,
/// `apply_phase10c_schema`, …). The partial UNIQUE indexes on the
/// secondary id columns enforce the cross-backend mapping
/// invariant: two distinct `event_id`s cannot claim the same
/// Vectorizer / Meili / Nexus native id.
///
/// Archive partition does NOT carry a UNIQUE index because many
/// envelopes share the same partition file (the canonical archive
/// layout is `events/year=…/month=…/day=…/hour=…/raw-NNNNN.parquet`
/// and each hour-bucket holds the entire batch).
pub fn apply_phase13d_schema(conn: &Connection) -> Result<(), IdentityError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_identity (
            event_id          TEXT PRIMARY KEY,
            nexus_id          TEXT,
            vec_id            TEXT,
            meili_id          TEXT,
            archive_partition TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_event_identity_nexus
            ON event_identity (nexus_id) WHERE nexus_id IS NOT NULL;
        CREATE UNIQUE INDEX IF NOT EXISTS idx_event_identity_vec
            ON event_identity (vec_id) WHERE vec_id IS NOT NULL;
        CREATE UNIQUE INDEX IF NOT EXISTS idx_event_identity_meili
            ON event_identity (meili_id) WHERE meili_id IS NOT NULL;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_phase13d_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_then_lookup_returns_full_identity_row() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        idx.upsert_identity("01EVT", Backend::Vectorizer, "vec-1")
            .unwrap();
        let row = idx.lookup("01EVT").unwrap().unwrap();
        assert_eq!(row.event_id, "01EVT");
        assert_eq!(row.vec_id.as_deref(), Some("vec-1"));
        assert!(row.nexus_id.is_none());
        assert!(row.meili_id.is_none());
        assert!(row.archive_partition.is_none());
    }

    #[test]
    fn upsert_merges_across_backends_for_same_event() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        idx.upsert_identity("01EVT", Backend::Vectorizer, "vec-1")
            .unwrap();
        idx.upsert_identity("01EVT", Backend::Meili, "doc-7")
            .unwrap();
        idx.upsert_identity("01EVT", Backend::Nexus, "node-42")
            .unwrap();
        idx.upsert_identity(
            "01EVT",
            Backend::Archive,
            "events/year=2026/month=05/raw-00000.parquet",
        )
        .unwrap();
        let row = idx.lookup("01EVT").unwrap().unwrap();
        assert_eq!(row.vec_id.as_deref(), Some("vec-1"));
        assert_eq!(row.meili_id.as_deref(), Some("doc-7"));
        assert_eq!(row.nexus_id.as_deref(), Some("node-42"));
        assert_eq!(
            row.archive_partition.as_deref(),
            Some("events/year=2026/month=05/raw-00000.parquet")
        );
    }

    #[test]
    fn lookup_by_each_native_id_finds_the_row() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        idx.upsert_identity("01EVT", Backend::Vectorizer, "vec-1")
            .unwrap();
        idx.upsert_identity("01EVT", Backend::Meili, "doc-7")
            .unwrap();
        idx.upsert_identity("01EVT", Backend::Nexus, "node-42")
            .unwrap();

        let by_vec = idx
            .lookup_by_native(Backend::Vectorizer, "vec-1")
            .unwrap()
            .unwrap();
        assert_eq!(by_vec.event_id, "01EVT");

        let by_meili = idx
            .lookup_by_native(Backend::Meili, "doc-7")
            .unwrap()
            .unwrap();
        assert_eq!(by_meili.event_id, "01EVT");

        let by_nexus = idx
            .lookup_by_native(Backend::Nexus, "node-42")
            .unwrap()
            .unwrap();
        assert_eq!(by_nexus.event_id, "01EVT");
    }

    #[test]
    fn delete_drops_the_row_and_is_idempotent() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        idx.upsert_identity("01EVT", Backend::Meili, "doc-7")
            .unwrap();
        assert!(idx.lookup("01EVT").unwrap().is_some());
        idx.delete("01EVT").unwrap();
        assert!(idx.lookup("01EVT").unwrap().is_none());
        // Idempotent — a second delete of an already-absent row
        // does NOT return an error. `admin_forget` relies on this
        // so a retried forget after a crash mid-cascade is safe.
        idx.delete("01EVT").unwrap();
    }

    #[test]
    fn unique_index_rejects_two_events_claiming_the_same_native_id() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        idx.upsert_identity("01EVT_A", Backend::Vectorizer, "vec-1")
            .unwrap();
        // A second envelope cannot claim the same Vectorizer id —
        // the partial UNIQUE index on `vec_id` rejects the insert.
        // This is the structural invariant the doctor relies on to
        // skip duplicate-id sweeps.
        let err = idx
            .upsert_identity("01EVT_B", Backend::Vectorizer, "vec-1")
            .unwrap_err();
        assert!(
            matches!(err, IdentityError::Sqlite(_)),
            "UNIQUE violation must surface as IdentityError::Sqlite, got {err:?}"
        );
    }

    #[test]
    fn empty_event_id_is_rejected_at_validation_time() {
        let conn = open();
        let idx = SqliteIdentityIndex::new(&conn);
        let err = idx
            .upsert_identity("", Backend::Vectorizer, "vec-1")
            .unwrap_err();
        match err {
            IdentityError::EmptyId { field } => assert_eq!(field, "event_id"),
            other => panic!("expected EmptyId, got {other:?}"),
        }
        // Symmetric — empty native id also rejected.
        let err = idx
            .upsert_identity("01EVT", Backend::Vectorizer, "")
            .unwrap_err();
        match err {
            IdentityError::EmptyId { field } => assert_eq!(field, "native_id"),
            other => panic!("expected EmptyId(native_id), got {other:?}"),
        }
    }

    #[test]
    fn backend_as_str_and_all_stay_in_sync() {
        let labels: Vec<&'static str> = Backend::all().iter().map(|b| b.as_str()).collect();
        assert_eq!(labels, vec!["nexus", "vectorizer", "meili", "archive"]);
    }
}
