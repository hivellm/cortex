# VectorizerSync Operations

## Installation & Deployment

### End-User Installation

**Windows**:
- Download `.exe` installer from GitHub releases
- Run installer (installs to `C:\Users\<username>\AppData\Local\vectorizer-sync\`)
- Installer registers app for system tray integration

**macOS**:
- Download `.dmg` from releases
- Open DMG and drag app to Applications folder
- Grant privacy permissions (Files and Folders access) on first run

**Linux**:
- Download AppImage, .deb, or .rpm
- Install via package manager or run AppImage directly
- Manually grant file system permissions if needed

### Development Build

```bash
# Clone and install
git clone https://github.com/hivellm/vectorizer-sync.git
cd vectorizer-sync
npm install

# Development with hot reload
npm run dev

# Build for current platform
npm run build

# Build for all platforms
npm run build:all
```

## Docker Deployment (Not Recommended)

**Note**: VectorizerSync is a desktop app and doesn't run in Docker. However, developers can build inside containers:

```dockerfile
FROM node:20-alpine
WORKDIR /build
COPY . .
RUN npm install
RUN npm run build
# Outputs to dist/ directory
```

## Environment Variables (Planned)

**Not Currently Used** (OAuth integration pending):

```bash
# Future
HIVEHUB_CLIENT_ID=<OAuth client ID>
HIVEHUB_CLIENT_SECRET=<OAuth client secret>
HIVEHUB_API_URL=https://api.hivehub.cloud/v1
VECTORIZER_SYNC_DEBUG=true  # Enable verbose logging
```

**Current**: All configuration via UI (no CLI flags)

## Ports & Network

### No Server Ports

VectorizerSync is a desktop app; it does NOT bind to any ports. It communicates:
- **Outbound**: HTTPS to HiveHub Cloud API (port 443, future)
- **Local**: IPC between Electron main and renderer processes (no network)

### Network Requirements

- **Cloud Sync Enabled**: Requires internet connectivity
- **Local Export Only**: Works offline
- **HTTPS**: All API calls to HiveHub use HTTPS (no HTTP)
- **Rate Limiting**: Respects HiveHub API rate limits (exponential backoff on 429)

## Data Directories

```
Windows:
  Config & DB: C:\Users\<username>\vectorizer-sync\
  Logs:        C:\Users\<username>\vectorizer-sync\logs\

macOS:
  Config & DB: ~/.vectorizer-sync/
  Logs:        ~/.vectorizer-sync/logs/

Linux:
  Config & DB: ~/.vectorizer-sync/ or ~/.config/vectorizer-sync/
  Logs:        ~/.vectorizer-sync/logs/
```

## Logging

**Log Files**: Stored in `~/vectorizer-sync/logs/` (platform-specific)

**Log Levels**:
- **ERROR**: Sync failures, unrecoverable errors
- **WARN**: Quota warnings, exclusions, API errors
- **INFO**: Sync started/completed, file operations
- **DEBUG**: File hash calculations, database queries (if `VECTORIZER_SYNC_DEBUG=true`)

**Retention**: Manual cleanup by user (no auto-rotation)

## Performance Characteristics

| Metric | Target |
|--------|--------|
| **App Startup** | < 3 seconds |
| **File Change Detection** | < 5 seconds |
| **DB Query (simple)** | < 100ms |
| **Memory Usage** | < 500MB (normal operation) |
| **Database Size** | < 100MB (typical) |

## Monitoring & Diagnostics

### Health Checks

**User-Facing**:
- Sync status visible in UI (success/failure)
- Quota display in settings
- Notification list for alerts

**Developer**:
```bash
# Check logs
tail -f ~/.vectorizer-sync/logs/app.log

# Database integrity
sqlite3 ~/.vectorizer-sync/database.db "PRAGMA integrity_check;"

# Database size
ls -lah ~/.vectorizer-sync/database.db*
```

### Common Issues

| Issue | Cause | Resolution |
|-------|-------|-----------|
| **Files not syncing** | Cloud sync disabled or no auth | Enable cloud sync, authenticate |
| **Quota exceeded** | All cloud storage used | Upgrade plan or delete old files |
| **File change detection slow** | Too many files watched | Reduce project scope or exclusions |
| **High memory usage** | Large projects (1000+ files) | Split into smaller projects |
| **Database locked** | Concurrent access | Close app and retry |

## Backup & Recovery

### Database Backup

**Manual Backup**:
```bash
cp ~/.vectorizer-sync/database.db ~/.vectorizer-sync/database.db.backup
```

**Restore**:
```bash
cp ~/.vectorizer-sync/database.db.backup ~/.vectorizer-sync/database.db
```

### Configuration Export

Users can export all settings via UI:
- `File → Export Configuration` (JSON format)
- Includes: projects, workspace.yml versions, settings, notification history

### Recovery Strategy

1. **Corrupted Database**: Delete `database.db`, restart app (recreates schema)
2. **Lost Projects**: Re-import configuration JSON or re-add manually
3. **Lost Workspace YAML**: Regenerate by exporting from UI

## Security Considerations

### File System Access

- App requests permission to read watched directories
- No arbitrary write access outside designated project folders
- Symlinks followed (with loop detection)

### API Communications (Future)

- HTTPS only (no HTTP fallback)
- SSL certificate validation required
- OAuth 2.0 for authentication (no password storage)
- Access tokens short-lived; refresh tokens in OS keychain

### Data Privacy

- Local data stays local (never sent to HiveHub unless user enables cloud sync)
- No telemetry or analytics (planned: optional crash reporting)
- No third-party APIs except HiveHub

## Uninstall & Cleanup

**Windows**:
- Control Panel → Programs → Uninstall → Vectorizer Sync
- Optionally: Delete `C:\Users\<username>\vectorizer-sync\` folder (preserves db for reinstall)

**macOS**:
- Drag Vectorizer Sync.app to Trash
- Optionally: `rm -rf ~/.vectorizer-sync/`

**Linux**:
- `apt remove vectorizer-sync` or equivalent for your distro
- Optionally: `rm -rf ~/.vectorizer-sync/`

**Data Persistence**: Database and logs remain after uninstall (not auto-deleted); user must manually remove if desired
