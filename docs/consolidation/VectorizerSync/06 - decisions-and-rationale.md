# VectorizerSync Decisions & Rationale

## D1: Desktop App vs. Web App vs. CLI

**Decision**: Electron desktop application

**Rationale**:
- **File System Access**: Unrestricted local file I/O (required for workspace scanning)
- **System Integration**: Tray icon, system notifications, OS credential storage
- **Offline Capability**: Works without internet (local export always available)
- **Performance**: Direct file watching without polling
- **User Experience**: Native feel on each platform

**Alternative**: Web app (rejected due to sandboxing limitations and browser API gaps)

## D2: SQLite vs. File-Based or Cloud Storage

**Decision**: SQLite local database in user home directory

**Rationale**:
- **Persistence**: Survives app updates and reinstalls
- **Queryability**: Efficient filtering/sorting of projects and file metadata
- **Transactions**: Atomic sync state updates
- **Transactions**: Atomic sync state updates
- **Disk Usage**: Efficient BLOB storage for YAML snapshots
- **Privacy**: All data stays local (no cloud logs unless user enables cloud sync)

**Alternative**: File-based JSON (rejected due to poor concurrent access handling)

## D3: File Change Detection: Timestamp vs. Hash

**Decision**: Content hash-based (SHA256) with filesystem events

**Rationale**:
- **Accuracy**: Detects actual changes, ignores timestamp-only updates
- **Correctness**: Two identical files produce same hash (no spurious uploads)
- **Editor Idempotency**: Text editor saves that don't change content don't trigger sync
- **Trade-off**: Slightly slower than timestamps, but correctness > speed

**Implementation**: Chokidar for filesystem events + hash on detected changes

## D4: Sync Direction: Local → Cloud (One-Way Push)

**Decision**: Unidirectional push (local is source of truth)

**Rationale**:
- **Conflict Avoidance**: Single direction eliminates merge conflicts
- **User Mental Model**: "Export to cloud" not "sync with cloud"
- **Data Safety**: User's local files never overwritten by cloud
- **Simplicity**: No need for complex 3-way merge logic (yet)

**Future**: Two-way sync with manual conflict resolution UI (not in v0.1.0)

## D5: Quota Enforcement: Hard Gate vs. Warning

**Decision**: Hard gate (uploads blocked when quota exceeded) with escalating warnings

**Rationale**:
- **User Expectation**: Don't charge user for over-quota uploads
- **Data Integrity**: Prevent partial uploads or truncation
- **Plan Respect**: Enforce subscription tiers

**Warning Timeline**:
- 80% quota → warning notification
- 90% quota → warning notification + UI badge
- 100% quota → upload blocked, error message with upgrade link

## D6: Exclusion Rule Strategy: Allowlist vs. Blocklist

**Decision**: Blocklist (exclude known patterns; include everything else)

**Rationale**:
- **New Projects**: New project structures don't break by default
- **User-Added Patterns**: Additive exclusions (user can refine)
- **Reversibility**: Removing a pattern is safer than discovering you need it

**Built-in Exclusions** (hardcoded, non-removable):
- Node modules, build artifacts, common binary extensions
- > 100KB files (configurable threshold)

**User-Customizable**: Additional glob patterns in settings

## D7: Workspace.yml Format: Vectorizer-Compatible vs. Custom

**Decision**: Exact Vectorizer format compliance

**Rationale**:
- **Interoperability**: Users can take exported YAML and use with any Vectorizer instance
- **Zero Adaptation**: No translation layer needed
- **Long-term**: If Vectorizer format evolves, VectorizerSync just regenerates

**Consequence**: If Vectorizer format changes, VectorizerSync must be updated

## D8: Dual Workspace Types: Local vs. Remote

**Decision**: Support both local (file export) and remote (HiveHub Cloud) workspaces

**Rationale**:
- **Flexibility**: Users with on-prem Vectorizer or those wanting cloud both supported
- **Migration Path**: Start with local, upgrade to cloud later
- **Offline**: Local workspace always works without internet

**Trade-off**: Extra complexity in UI and sync logic

## D9: Configuration Persistence: JSON + SQL vs. Single Format

**Decision**: JSON import/export + SQL internal storage

**Rationale**:
- **User Portability**: JSON configurations are human-readable and transferable
- **Internal Efficiency**: SQL queries on settings and projects
- **No Format Mismatch**: Users don't see SQL; they interact with JSON

**Migration**: Import JSON → parse → store in SQL on first load

## D10: Notification Storage: In-Database vs. External

**Decision**: In-database with user-managed retention

**Rationale**:
- **Consistency**: All state in one place
- **Queryability**: Filter by type/severity/date
- **No External Service**: No cloud dependency for local notifications

**Consequence**: Users manually clear old notifications (no auto-purge planned)

## D11: API Token Security: OS Keychain vs. File Encryption

**Decision**: OS keychain/credential store (when implemented)

**Rationale**:
- **Platform Standards**: Leverages OS security infrastructure
- **User Expectation**: OAuth tokens not in plaintext database
- **Portable**: Credentials tied to user account, not machine

**Platforms**:
- Windows: Credential Manager
- macOS: Keychain
- Linux: Secret Service (dbus)

## D12: Sync Batching: Queue-Based vs. Real-Time Upload

**Decision**: Queue with debouncing (batch rapid changes)

**Rationale**:
- **Performance**: 100 file changes in 1 second → 1 batch, not 100 API calls
- **Quota Efficiency**: Fewer round-trips = fewer request charges (if pay-per-call)
- **Network**: Bulk uploads more efficient than streaming
- **Debounce Window**: 5–10 seconds (TBD in implementation)

## D13: Electron Builder for Packaging

**Decision**: Electron Builder for cross-platform installers

**Rationale**:
- **Consistency**: Same tooling across Windows, macOS, Linux
- **Code Signing**: Built-in support for Windows and macOS certificates
- **Auto-Update**: Foundation for future auto-updater feature
- **Distribution**: Generates installers, DMG, AppImage, .deb, .rpm

## Deferred Decisions (v0.2.0+)

1. **Two-Way Sync**: Requires UI for conflict resolution
2. **Workspace Versioning**: Keep history of exported YAML
3. **File Versioning**: Version history per file (HiveHub feature)
4. **Advanced Conflict UI**: Manual merge/resolve for user conflicts
5. **Bandwidth Throttling**: Rate-limit uploads
6. **Encryption**: End-to-end encryption for cloud-stored files
7. **Auto-Updater**: Self-updating app (Electron-updater integration)
