# VectorizerSync Architecture

## System Components

### 1. Database Manager
- **Location**: `src/main/database/`
- **Responsibility**: SQLite operations, schema management, migrations
- **Data Stored**: Projects, workspace configs, sync history, file metadata, notifications, user settings
- **Database Location**: 
  - Windows: `C:\Users\<username>\vectorizer-sync\database.db`
  - macOS/Linux: `~/vectorizer-sync/database.db`

### 2. File System Watcher
- **Location**: `src/main/watcher/`
- **Technology**: Chokidar
- **Responsibility**: Monitor watched directories for file additions, modifications, deletions
- **Performance**: < 5 second detection latency
- **Debouncing**: Handles rapid file changes through batching

### 3. Sync Engine
- **Location**: `src/main/sync/`
- **Responsibility**:
  - Export `workspace.yml` in Vectorizer format
  - Queue and process file changes
  - Upload/update files to HiveHub Cloud (when enabled)
  - Handle sync conflicts
  - Enforce quota limits
- **Operations**: Export, upload, update, delete (tracked per operation)

### 4. HiveHub API Client
- **Location**: `src/main/api/`
- **Status**: Planned (not yet implemented)
- **Responsibility**: 
  - OAuth 2.0 authentication (future)
  - File upload/download
  - Quota and account management
  - Notification retrieval
  - Error handling and retry logic (exponential backoff)

### 5. Notification System
- **Location**: `src/main/` + `src/renderer/`
- **Types**:
  - **Internal**: Sync status, errors, exclusions
  - **HiveHub**: Quota warnings, service alerts
- **Display**: In-app notification center + OS-level system tray alerts

## Data Flow

```
Local Filesystem
    ↓
Chokidar File Watcher (detects changes)
    ↓
Change Detection (file hashing, debounce)
    ↓
Sync Queue (batch operations)
    ↓
Database Manager (persist state)
    ↓
Sync Engine (process queue)
    ↓
HiveHub API Client (if cloud sync enabled)
    ↓
HiveHub Cloud / Local Vectorizer
```

## Core Data Models

### Project
```
id (UUID)
name, path (unique)
workspace_type: 'local' | 'remote'
workspace_path? (for local)
cloud_workspace_id? (for remote)
sync_enabled
last_sync_at
created_at, updated_at
```

### File Metadata
```
id (UUID)
project_id
relative_path, absolute_path (unique per project)
size, hash (SHA256 for change detection)
last_modified, synced_at
sync_status: 'pending' | 'synced' | 'failed' | 'excluded'
exclusion_reason?
```

### Sync History
```
id (UUID)
project_id
type: 'export' | 'upload' | 'update' | 'delete'
status: 'success' | 'failed' | 'partial'
files_count, files_processed
errors (JSON array)
started_at, completed_at
```

## Process Architecture

- **Main Process**: Handles database, file watching, sync queue processing (non-blocking)
- **Renderer Process**: React UI (project selection, settings, status display)
- **IPC Communication**: Sync status updates, command dispatching, state synchronization
- **No Blocking Operations**: Sync operations run in background; UI remains responsive

## Key Design Decisions

1. **SQLite Local Storage**: Data persists in user home directory, survives app updates
2. **Hash-Based Change Detection**: Detect actual content changes, not just timestamps
3. **Batch Sync Queue**: Optimize by bundling rapid file changes
4. **Plan Limits as Hard Gate**: Uploads blocked when quota exceeded
5. **Dual Workspace Modes**: Support local Vectorizer instances AND HiveHub Cloud
