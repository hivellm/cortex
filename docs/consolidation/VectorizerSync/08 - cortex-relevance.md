# VectorizerSync Cortex Integration Relevance

## Cortex Consolidation Priority

**Classification**: LOW-TO-MEDIUM priority for Cortex ingestion

**Rationale**: VectorizerSync is a desktop client tool (not a backend service). Cortex is designed to capture and consolidate backend knowledge. VectorizerSync's knowledge graph is useful for:
1. Understanding how local projects map to Vectorizer workspaces
2. Tracking file-to-index relationships
3. Identifying sync state anomalies

## Ingestion Strategy for Cortex

### High-Priority Ingestion

1. **Workspace Format Specification**
   - Node Type: `WorkspaceSchema` / `VectorizerFormat`
   - Attributes: YAML structure, field types, required/optional
   - Relationship: `VectorizerSync → exports → WorkspaceYAML`
   - **Use Case**: When Cortex ingests a Vectorizer workspace, it needs to understand schema origins

2. **File Filtering Rules**
   - Node Type: `ExclusionRule` (global set)
   - Attributes: Pattern, priority, rationale
   - Relationship: `Project → applies → ExclusionRules`
   - **Use Case**: When Cortex indexes a project, it can apply same exclusions as VectorizerSync did

3. **Project Directory Mapping**
   - Node Type: `Project` (with workspace_type tag)
   - Attributes: Local path, workspace type, last sync timestamp
   - Relationship: `LocalDirectory ↔ Project ↔ Workspace`
   - **Use Case**: Cortex can cross-reference local projects with cloud workspaces

### Medium-Priority Ingestion

4. **Sync History & Reliability**
   - Node Type: `SyncOperation` (with status)
   - Attributes: Timestamp, file count, success rate
   - Relationship: `Project → has_sync_history → SyncOperation`
   - **Use Case**: Cortex understands which projects sync reliably vs. frequently fail

5. **Quota & Plan Information**
   - Node Type: `Account` / `Quota`
   - Attributes: Plan type, quota limit, quota used
   - Relationship: `User → has_account → HiveHub; Account → has_quota → Quota`
   - **Use Case**: Cortex respects user's cloud storage limits during ingestion tasks

### Low-Priority Ingestion

6. **File Metadata Hash Index** (Optional)
   - Node Type: `FileHash`
   - Attributes: SHA256, file path, project ID
   - Relationship: `File → has_hash → FileHash`
   - **Use Case**: Deduplication across projects or comparing with Vectorizer index

## Data Contracts for Cortex

### Input from VectorizerSync to Cortex

**When a user enables VectorizerSync-to-Cortex integration**:

```json
{
  "source": "vectorizer-sync",
  "projects": [
    {
      "id": "uuid",
      "name": "string",
      "path": "/absolute/path",
      "workspace_type": "local|remote",
      "cloud_workspace_id": "string?",
      "last_sync_at": "ISO8601",
      "file_count": 1234,
      "excluded_files_count": 567,
      "workspace_yml": "<yaml>"
    }
  ],
  "user_settings": {
    "max_file_size": 102400,
    "exclusion_patterns": ["pattern1", "pattern2"],
    "quota": {
      "limit": 1099511627776,
      "used": 536870912
    }
  },
  "sync_statistics": {
    "total_syncs": 42,
    "successful_syncs": 40,
    "last_sync_timestamp": "ISO8601"
  }
}
```

### Output from Cortex to VectorizerSync (Future)

**Cortex could report back**:

```json
{
  "target": "vectorizer-sync",
  "recommendations": [
    {
      "project_id": "uuid",
      "type": "exclusion_suggestion",
      "pattern": "*.generated.ts",
      "rationale": "File type never indexes successfully"
    }
  ],
  "indexing_status": {
    "project_id": "uuid",
    "indexed_files": 1000,
    "indexing_progress": 95,
    "errors": [
      {
        "file": "path/to/file",
        "error": "parsing failed"
      }
    ]
  }
}
```

## Knowledge Graph Entities

**VectorizerSync Nodes**:
- `VectorizerSync` (app/tool)
- `Project` (local project with workspace mapping)
- `Workspace` (Vectorizer workspace, local or remote)
- `WorkspaceYAML` (exported config)
- `FileFilter` / `ExclusionRule`
- `SyncOperation` (audit trail)
- `HiveHubAccount` / `Quota`

**Relationships**:
- `VectorizerSync → manages → Project`
- `Project → exports → WorkspaceYAML`
- `Project → applies → ExclusionRule`
- `Project → syncs_to → Workspace`
- `Project → has_sync_history → SyncOperation`
- `User → owns → HiveHubAccount`
- `HiveHubAccount → manages → Quota`

## Ingest Priority Timeline

| Phase | Component | Reason |
|-------|-----------|--------|
| **Phase 1** (Now) | Workspace format, exclusion rules | Core to understanding any Vectorizer integration |
| **Phase 2** (After cloud sync shipping) | Sync history, quota tracking | Understand user's sync behavior and limits |
| **Phase 3** (Post-beta) | File metadata hash index | Advanced: deduplication and integrity checks |

## Cortex Use Cases Enabled

1. **Cross-Project Deduplication**: Cortex sees file hashes from VectorizerSync and can warn about duplicates across projects
2. **Exclusion Rule Audits**: Cortex can analyze whether excluded files should have been included
3. **Sync Reliability Alerts**: Cortex notices patterns of failed syncs and suggests remediation
4. **Workspace Consistency Checks**: Cortex compares exported YAML vs. actual file system state
5. **Quota Forecasting**: Cortex tracks growth rate and warns before quota exceeded

## Non-Relevant Aspects

- **UI/UX Details**: Cortex doesn't need to know about React components or Electron IPC
- **Cross-Platform Quirks**: File path normalization is implementation detail
- **Development Tooling**: Electron Builder, TypeScript config not relevant to Cortex
- **Internal State Management**: React hooks, Zustand stores not relevant

## Recommendation

**For Initial Cortex Consolidation**:
1. Ingest `workspace.yml` format spec → WorkspaceSchema nodes
2. Ingest exclusion rules as global ExclusionRule nodes
3. Tag with `source:vectorizer-sync` for traceability
4. Defer file-level metadata until cloud sync is live and stable
