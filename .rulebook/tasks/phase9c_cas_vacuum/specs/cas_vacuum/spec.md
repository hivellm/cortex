# Spec: CAS vacuum

## ADDED Requirements

### Requirement: Periodic vacuum of unreferenced blobs

The `cortex-retention cas-vacuum` command MUST delete every row from
`cas_blobs` where `refcount = 0 AND last_referenced < now - min_age_days`.

The default `min_age_days` MUST be 30 unless overridden in
`cortex.toml [retention.cas]`.

#### Scenario: orphan blob older than 30 days is deleted
Given `cas_blobs` has a row with `refcount=0` and `last_referenced=now-31d`
When `cortex-retention cas-vacuum` runs
Then the row MUST be deleted
And the report MUST include `blobs_dropped >= 1`.

#### Scenario: referenced blob is preserved
Given a row with `refcount=2`
When the vacuum runs
Then the row MUST remain.

### Requirement: SQLite reclamation

When the metadata DB has more than 25% free pages after deletes, the
runner MUST execute `VACUUM` (or `VACUUM INTO` followed by atomic swap)
to reclaim disk space.

#### Scenario: disk shrinks after a large vacuum
Given the metadata file is 100 MB with 40 MB of free pages after delete
When the vacuum completes
Then the metadata file size on disk MUST decrease by ≥ 30 MB.

### Requirement: Refcount audit

`cortex-retention cas-vacuum --audit` MUST recompute `refcount` for every
hash by enumerating CAS references in Vectorizer, Nexus, and Meili
payloads. The audit MUST report drift without mutating unless `--fix`
is supplied.

#### Scenario: --fix repairs an under-counted blob
Given a blob whose stored refcount is 1 but real references count 3
When `cas-vacuum --audit --fix` runs
Then `cas_blobs.refcount` for that hash MUST be 3.

### Requirement: Catastrophic-deletion safeguard

The vacuum MUST refuse to delete if the candidate set covers more than
50% of total blobs unless `--force` is passed.

#### Scenario: refcount bug zeroes most rows, vacuum refuses
Given `cas_blobs` has 1000 rows and 600 are vacuum candidates
When `cas-vacuum` runs without `--force`
Then it MUST exit non-zero with a "safeguard tripped" message
And it MUST NOT delete any rows.
