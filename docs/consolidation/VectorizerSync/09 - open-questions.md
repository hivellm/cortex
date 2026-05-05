# VectorizerSync Open Questions & Gaps

## Design Gaps

### G1: Two-Way Sync Conflict Resolution
**Q**: When Vectorizer (or user manually) modifies a synced file in HiveHub Cloud, should VectorizerSync pull it back to local?

**Current Plan**: No (local is always source of truth)

**Open Issue**:
- What if user deletes file locally but cloud version has newer content?
- Manual conflict resolution UI not designed yet
- Deferred to v0.2.0+

**Decision Needed**: Scope for v0.1.0 (local-push-only) or add pull+conflict-UI?

---

### G2: Partial Sync Failure Recovery
**Q**: If a sync of 1000 files fails on file 500, should we:
1. Retry all 1000?
2. Resume from file 501?
3. Mark as failed and require manual retry?

**Current**: No detailed recovery strategy documented

**Impact**: Large projects with frequent network interruptions

---

### G3: File Versioning
**Q**: Should VectorizerSync preserve file version history in HiveHub Cloud?

**Current**: Not in scope (cloud stores only latest version)

**Future**: Possible feature if HiveHub API supports versioning

**Implication**: Users can't revert synced files to earlier versions

---

### G4: Shared Workspaces / Multi-User
**Q**: Can multiple users sync the same project to the same HiveHub workspace without conflicts?

**Current**: Not supported (no access control in design)

**Future**: Possible but requires locking/CRDT/conflict resolution

---

### G5: Symlink Handling
**Q**: How should VectorizerSync handle symlinks in watched directories?

**Scenarios**:
- Symlink to external project (should we follow?)
- Symlink to sister project (infinite loop risk?)
- Symlink to shared library (duplicate syncing?)

**Current**: Mentioned (handle symlinks appropriately) but no detailed rules

**Need**: Clear policy and loop detection

---

## Implementation Uncertainties

### U1: HiveHub API Stability
**Q**: What is the final API contract for file uploads to HiveHub Cloud?

**Documented**: OAuth + REST endpoints (future)

**Risk**: API may change before implementation; endpoint signatures unknown

**Mitigation**: Design with abstraction layer; mock API in tests

---

### U2: Database Query Performance at Scale
**Q**: What's the practical limit on projects/files VectorizerSync can handle?

**Tested**: Unknown (no benchmarks exist yet)

**Risk**: 100,000+ files in one project → slow indexing or UI lag

**Unknowns**:
- SQLite concurrency under rapid file changes
- Memory footprint with massive file metadata table
- Query performance on `file_metadata` with millions of rows

**Need**: Load testing before v1.0 release

---

### U3: Quota Enforcement with Offline Mode
**Q**: If user goes offline during sync, how do we prevent quota overages when connectivity returns?

**Scenario**: User was at 95% quota, syncs 100 files offline, comes back online with no quota

**Current**: No strategy for handling stale quota information

**Options**:
1. Check quota before every batch (safe but slow)
2. Warn user at 80%/90% and let them manage
3. Prevent uploads if quota unavailable (safest but might frustrate users)

---

### U4: File Hashing Performance
**Q**: What's the performance impact of SHA256 hashing 10,000 files every sync?

**Current**: No optimization strategy documented

**Risk**: File hashing could dominate sync duration for large projects

**Possible Optimizations**:
- Incremental hashing (only on recently modified files)
- Parallel hashing (if file watching reports timestamps)
- Mtime-based change detection with hash verification

---

### U5: Cross-Platform Path Normalization
**Q**: How do we ensure `workspace.yml` paths work consistently across Windows, macOS, Linux?

**Current**: "Handle platform-specific file path formats" documented but no implementation detail

**Edge Cases**:
- Absolute Windows path in workspace.yml on macOS?
- UNC paths on network drives?
- Relative paths vs. absolute?

**Need**: Concrete path resolution rules per platform

---

## Unspecified Behaviors

### B1: Database Migration Timeline
**Q**: What happens if user has v0.1.0 database and upgrades to v0.2.0 with schema changes?

**Current**: Migration framework exists but no real migrations specified

**Gap**: Version-to-version compatibility not addressed

---

### B2: Notification Cleanup
**Q**: Do notifications ever auto-delete or accumulate forever?

**Current**: Manual user cleanup only

**Risk**: Database grows unbounded with old notifications

**Need**: Retention policy (e.g., delete > 30 days old)

---

### B3: Error Message Clarity
**Q**: What specific error messages should users see for common failures?

**Examples**:
- "Sync failed" — too generic
- "File too large (152KB > 100KB limit)" — better

**Current**: Generic error structure exists but actual messages not defined

---

### B4: Workspace.yml Diff & History
**Q**: Should VectorizerSync track differences between workspace.yml versions?

**Current**: Multiple versions stored but no diffing logic

**Future**: Could show user "what changed since last export"

---

### B5: Sync Cancellation
**Q**: What happens if user clicks "Cancel Sync" mid-operation?

**Incomplete Sync States**:
- Files uploaded: 500/1000
- Cloud now inconsistent with local?
- What gets rolled back?

**Current**: No cancellation UI or strategy documented

---

## External Dependencies Unknowns

### D1: Vectorizer Workspace Format Evolution
**Q**: If Vectorizer's `workspace.yml` schema changes, how do we update VectorizerSync?

**Risk**: Forward/backward compatibility breaks

**Current**: No versioning strategy in workspace.yml schema

---

### D2: HiveHub API Rate Limiting
**Q**: What are the rate limits on HiveHub Cloud API?

**Impact**: Affects batch size, retry strategy, backoff timing

**Undocumented**: No API rate limits specified

---

### D3: OS Keychain Availability
**Q**: What if user's OS keychain/credential store is unavailable or corrupted?

**Future Scenario**: OAuth tokens stored in keychain; keychain fails

**Fallback**: No documented fallback strategy

---

## Testing Gaps

### T1: Cross-Platform File Path Tests
**Q**: Are tests running on actual Windows, macOS, and Linux file systems?

**Risk**: Path handling bugs only visible on specific OS

**Current**: Test strategy planned but not detailed

---

### T2: Large-Scale Project Tests
**Q**: Do tests exercise projects with 10,000+ files?

**Current**: Not mentioned in testing strategy

**Need**: E2E tests with realistic large projects

---

### T3: Network Failure Recovery Tests
**Q**: Are tests for transient network failures (timeout, 503, etc.) comprehensive?

**Current**: Generic error handling, but detailed retry scenarios not specified

---

## Recommendations for Resolution

| Gap | Owner | Timeline | Action |
|-----|-------|----------|--------|
| G1 (Two-way sync) | Design | v0.2.0 planning | Schedule design review |
| U2 (Scale perf) | Dev | v0.1.0 beta | Benchmark with 100K files |
| U5 (Path normalization) | Dev | v0.1.0 alpha | Write path resolution spec |
| B3 (Error messages) | UX | v0.1.0 alpha | Define error catalog |
| D1 (Format evolution) | Arch | Ongoing | Monitor Vectorizer releases |
| T2 (Large-scale tests) | QA | v0.1.0 beta | Add load test suite |

## Decision Point: v0.1.0 Release Criteria

Before shipping v0.1.0 beta:
1. Resolve G2 (partial sync recovery) → implement strategy
2. Resolve U4 (hash performance) → benchmark or optimize
3. Resolve B5 (sync cancellation) → implement or explicitly forbid
4. Complete T2 (large-scale tests) → prove scalability
5. Finalize D2 (rate limit docs from HiveHub team) → adjust retry logic accordingly
