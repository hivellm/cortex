# VectorizerSync Data & Storage

## Primary Storage: SQLite

**Location**: User home directory (platform-specific)
- Windows: `C:\Users\<username>\vectorizer-sync\database.db`
- macOS: `~/vectorizer-sync/database.db`
- Linux: `~/vectorizer-sync/database.db`

**File Size**: Grows with project count and sync history depth; expected < 100MB for typical usage

**Backup**: Manual user responsibility; database is portable across systems

## Schema Overview

### Core Tables

**projects** (7 columns)
- Stores project metadata and configuration
- Unique constraint on `path` (only one project per directory)
- Indexed on: `path`, `sync_enabled`, `workspace_type`

**file_metadata** (10 columns)
- Tracks all files in watched projects
- Hash-based change detection (SHA256)
- Sync status tracking: pending | synced | failed | excluded
- Indexed on: `project_id`, `sync_status`, `hash`

**sync_history** (9 columns)
- Audit trail of all sync operations
- Tracks: export, upload, update, delete operations
- Partial failure tracking (files_processed vs. files_count)
- Indexed on: `project_id`, `started_at`

**workspace_configs** (5 columns)
- Versioned snapshots of exported `workspace.yml`
- Multiple versions per project (history)
- Full YAML content stored as TEXT (JSON-serializable)

**notifications** (7 columns)
- Internal + HiveHub notifications
- Indexed on: `read` status, `created_at`
- Retention: Manual user cleanup (no auto-purge)

**user_settings** (1 singleton row, id='default')
- Global application settings
- Sync preferences (enabled, interval, auto-sync flag)
- Max file size threshold (default: 102400 bytes = 100KB)
- Exclusion patterns (stored as JSON array)
- Notification preferences (severity filters)
- HiveHub account info (OAuth token details when connected)

## Replication State Management

### Sync State Tracking

**For Each File**:
```typescript
{
  sync_status: 'pending' | 'synced' | 'failed' | 'excluded',
  hash: string (SHA256),
  last_modified: timestamp,
  synced_at?: timestamp,
  size: number,
  exclusion_reason?: string
}
```

**Per-Project Tracking**:
- `last_sync_at`: Timestamp of most recent successful full sync
- `sync_enabled`: Whether cloud sync is active for this project

**Global Tracking**:
- Sync interval (minutes between auto-syncs)
- Auto-sync enabled flag

### Change Detection

1. **File System Event** → Chokidar detects add/mod/delete
2. **Hash Calculation** → SHA256 hash of file content
3. **Database Lookup** → Compare hash against stored hash
4. **If Changed** → Mark status='pending', queue for sync
5. **Sync Processing** → Upload/update/delete in cloud, mark status='synced'
6. **Failure** → Retry logic (exponential backoff), mark status='failed', log error

### Conflict Resolution

**Local-to-Cloud Conflicts** (Future Implementation):
- Last-write-wins (timestamp comparison)
- Manual resolution UI for high-value files (feature TBD)

**Current Behavior**: Overwrites cloud version with local (local is source of truth)

## Quota Management Storage

**In user_settings**:
```json
{
  "hivehub_account": {
    "email": "user@example.com",
    "planType": "starter|pro|enterprise",
    "quotaLimit": 1099511627776,     // bytes
    "quotaUsed": 536870912           // bytes
  }
}
```

**Update Frequency**: Synced with HiveHub API before each upload operation

**Enforcement**: Uploads blocked if `quotaUsed >= quotaLimit` (with warnings at 80%, 90%)

## Transactional Integrity

**Database Transactions**:
- Bulk file inserts/updates wrapped in transactions
- Sync history recorded atomically with file metadata updates
- Settings changes transactional (no partial states)

**WAL Mode**: SQLite in WAL (Write-Ahead Logging) mode for better concurrency

## Data Cleanup

**User Home Directory Structure**:
```
~/vectorizer-sync/
├── database.db        (SQLite main)
├── database.db-shm   (Write-Ahead Log shared memory)
├── database.db-wal   (Write-Ahead Log journal)
└── logs/             (application logs, manual cleanup)
```

**No Automatic Purging**: Users manage notification history and logs manually

## Encryption & Security

**At Rest**: SQLite database in plaintext (encrypted via filesystem volume encryption recommended)

**API Tokens** (Future): Stored in OS keychain (Credential Manager on Windows, Keychain on macOS, Secret Service on Linux)

**OAuth Tokens**: Refresh tokens persisted; access tokens regenerated on use

## Migration & Versioning

**Schema Version Table**:
```sql
CREATE TABLE schema_version (
  version INTEGER PRIMARY KEY,
  applied_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

**Migration Strategy**: On app startup, compare current schema version vs. database version; run pending migrations sequentially

**Current Version**: 1 (planning for extensibility)
