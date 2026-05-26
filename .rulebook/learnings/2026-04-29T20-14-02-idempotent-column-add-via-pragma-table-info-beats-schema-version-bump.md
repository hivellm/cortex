# Idempotent column-add via PRAGMA table_info beats SCHEMA_VERSION bump
**Source**: manual
**Date**: 2026-04-29
**Related Task**: phase9a_retention_sweeper_core
**Tags**: sqlite, migration, phase9a, schema-evolution
When adding a column to a table that already exists in deployed databases, do NOT bump `SCHEMA_VERSION` and reject incompatible DBs (the existing migrate() flow does that, breaking every deployed cortex-api). Instead probe `pragma_table_info(<table>) WHERE name = '<col>'`, ALTER TABLE if absent, leave SCHEMA_VERSION alone. Pre-phase9a databases get the new `retention_sweeps.status` column on their next boot without manual intervention. Pattern works because SQLite's `ALTER TABLE ADD COLUMN` is idempotent under a PRAGMA-gate guard. Apply the same trick when phase9b/d/g need their own table extensions.