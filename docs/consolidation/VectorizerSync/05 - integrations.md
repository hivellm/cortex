# VectorizerSync Integrations

## Vectorizer Dependency

### Vectorizer Workspace Format

VectorizerSync exists primarily to generate and maintain `workspace.yml` files compatible with Vectorizer (local or cloud).

**Export Format**: YAML, follows Vectorizer specification exactly
```yaml
global_settings:
  file_watcher:
    enabled: true
    auto_discovery: true
    exclude_patterns: []
    hot_reload: true
    watch_paths: [<absolute-paths>]

projects:
  - name, path, description
    collections:
      - name, include_patterns, exclude_patterns
```

**Vectorizer Consumption**: Local Vectorizer instances read exported `workspace.yml` to auto-discover projects and files for indexing

**Integration Point**: File list must be accurate; changes to exclusion rules must be propagated to workspace.yml exports

### Vectorizer SDK Usage

**Current Plan**: None (VectorizerSync is a standalone desktop app, not a Vectorizer client library)

**Future Possibility**: If Vectorizer SDK supports workspace management APIs, VectorizerSync could validate workspace.yml against Vectorizer schema programmatically

## HiveHub Cloud Integration

### Status: Documented Only (Not Implemented)

**Authentication**: OAuth 2.0 (planned)
- Redirect URI: `vectorizer-sync://oauth/callback`
- Scopes: `workspace:read`, `workspace:write`, `files:read`, `files:write`
- Token storage: OS keychain (secure)

### API Endpoints (Planned)

**File Operations**:
```
POST   /api/v1/workspaces/{id}/files        (upload)
PUT    /api/v1/workspaces/{id}/files/{fid}  (update)
DELETE /api/v1/workspaces/{id}/files/{fid}  (delete)
GET    /api/v1/workspaces/{id}/files        (list)
```

**Account**:
```
GET /api/v1/account/quota
GET /api/v1/workspaces
GET /api/v1/notifications
```

### Cloud Sync Workflow

1. User enables cloud sync for a project
2. VectorizerSync authenticates via OAuth (if not already authenticated)
3. User selects or creates a HiveHub Cloud workspace
4. VectorizerSync maps project → cloud workspace ID
5. File changes trigger uploads
6. Quota checked before each batch upload
7. Failures logged; user notified

### Notification Push (Future)

HiveHub may push notifications to VectorizerSync (via WebSocket or polling):
- Quota warnings (80%, 90%, exceeded)
- Service maintenance alerts
- Security notifications
- Collaboration invites (if multi-user enabled in future)

**Current Plan**: Polling-based (app checks `/notifications` periodically)

## Operating System Integration

### Windows
- **Data Location**: `C:\Users\<username>\AppData\Local\vectorizer-sync\`
- **Installer**: `.exe` (via Electron Builder)
- **System Tray**: Native Windows tray integration
- **Credentials**: Windows Credential Manager (for OAuth tokens)
- **File Watching**: Native Win32 change notifications (via Chokidar)

### macOS
- **Data Location**: `~/vectorizer-sync/` and `~/Library/Application Support/vectorizer-sync/`
- **Installer**: `.dmg` (via Electron Builder)
- **System Tray**: macOS menu bar integration
- **Credentials**: macOS Keychain
- **Permissions**: Privacy permissions for file system access (required)
- **Code Signing**: Certificate-based (planned for production)

### Linux
- **Data Location**: `~/.config/vectorizer-sync/` and `~/vectorizer-sync/`
- **Package**: AppImage, .deb, .rpm (via Electron Builder)
- **System Tray**: Desktop environment-agnostic (freedesktop.org standards)
- **Credentials**: Secret Service dbus API
- **Permissions**: Standard Unix file permissions

## File Filtering Dependencies

**Exclude Patterns** (built-in, non-negotiable):
- Node.js: `node_modules/`, `.npm/`, `.pnpm-store/`
- Python: `venv/`, `.venv/`, `__pycache__/`, `env/`
- Rust: `target/`, `.cargo/`
- Build: `dist/`, `build/`, `out/`, `.next/`, `.nuxt/`
- IDE: `.vscode/`, `.idea/`, `.workspace/`
- Git: `.git/`, `.gitignore` (preserved)
- Size: > 100KB (configurable, max 10MB)

**User-Customizable**: Additional glob patterns via settings UI

## External Dependencies

### NPM/Node Ecosystem
- **Electron**: Desktop framework (IPC, file dialogs, system tray)
- **React**: UI framework (state management, components)
- **Chokidar**: File system watcher (cross-platform)
- **YAML**: `workspace.yml` parsing/generation
- **Better-sqlite3**: SQLite driver (sync operations)

### OAuth & Crypto (Future)
- OAuth 2.0 client library (for HiveHub auth)
- `crypto` (Node.js built-in): SHA256 hashing for file change detection

### No Direct Dependencies on Other HiveLLM Services
- VectorizerSync is standalone
- Does NOT depend on Vectorizer, Nexus, Synap, or Lexum SDKs
- Only reads Vectorizer's workspace format; does not call Vectorizer APIs (currently)

## Data Contracts

### Input: Local Filesystem
- Read directories (recursive traversal)
- Read file content (for hashing)
- Watch for changes (via OS file system events)

### Output: workspace.yml
- YAML file written to user-selected location or HiveHub Cloud
- Must be parseable by Vectorizer

### Input: HiveHub API Responses (Future)
- OAuth tokens
- Workspace metadata
- Quota information
- Notifications

### Output: File Uploads
- Multipart/form-data (file content)
- Metadata (path, size, hash)
