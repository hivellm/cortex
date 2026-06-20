# 05 — Cross-domain graph: code + DB schema + infra — **MED-HIGH**

## What graphify does

One graph spans **app code + database schema + infrastructure + docs**, with edges crossing the boundaries:
- `pg_introspect.py`: connects to a **live Postgres**, queries `information_schema` to reconstruct DDL (tables, views, FKs, functions), then runs it through the SQL extractor → table/column/view nodes + `foreign_key_to` edges. (Captures *running* schema, not a guess from migration files.)
- `extract_sql()` (tree-sitter-sql): `.sql`/`.ddl` files → tables, views, `has_column`, `foreign_key_to`.
- `cargo_introspect.py`: walks Cargo workspace members → `crate_depends_on` edges.
- Semantic (LLM) extraction over prose links domains: "endpoint calls user service which queries the User table" → cross-domain `queries` edges.
- Leiden then forms communities that **span** code+schema+infra (e.g. `{OrderService, OrderTable, CustomerTable, db_module}`), so an agent gets code references + schema relationships + infra links in one query.

## What Cortex does today

- **No DB-schema ingestion** and **no infra ingestion** (grep: no `information_schema`, no pg/sql-schema introspection, no terraform). Cortex's graph is code symbols + sessions/turns/tool-calls + decisions/laws/memories.
- Cortex *does* ingest config/manifest files as content (tree-sitter `json`/`toml`), but not as a **schema/dependency graph** with FK/depends edges.

**Gap:** an agent asking "which code touches the `events` table" or "what depends on the Nexus service" can't be answered from the graph — the schema and the service topology aren't nodes.

## Recommendation for Cortex

Additive ingestion sources that emit into the existing graph projection — no architecture change:

1. **Cargo workspace topology** (cheapest, immediately useful on *this* repo): a small introspector that reads `Cargo.toml` workspace members + deps → `Crate` nodes + `crate_depends_on` edges. Cortex is a 14-crate workspace; this makes the dependency DAG (already a documented architecture concern) queryable and lintable for cycles.
2. **DB schema** (if/when a Cortex-managed relational store is in scope, or for the user's other projects): a `pg_introspect`-style source → `Table`/`Column`/`View` nodes + `foreign_key_to`/`has_column` edges, plus heuristic/LLM `queries` edges from code to tables.
3. **Service/infra topology:** parse `docker-compose.yml` (Cortex already has one) → `Service` nodes + `depends_on` edges; optionally Terraform/k8s later. This makes the running-system topology a first-class graph citizen alongside code.
4. **Cross-domain edges:** let the semantic analyzer emit `queries`/`deploys`/`configures` edges so communities (file 02) naturally span domains.

## Effort / impact

- **Cargo topology:** Impact MED (cycle detection + dependency queries on Cortex itself), Effort LOW. Good first slice.
- **DB schema + infra:** Impact MED-HIGH for systems with a DB/infra surface, Effort MED per source. Gate on a real consumer (don't build schema ingestion with no DB to point at).
- **Pairs with:** 02 (cross-domain communities), 04 (confidence on inferred `queries` edges).
