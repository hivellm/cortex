# Non-Code Parsers — Coverage Map for Cortex

UA ships 12 deterministic non-code parsers + ~40 language configs + framework registry, all feeding
the same node/edge schema. This extends the graph past source code into infra, data, and config —
a current Cortex blind spot. See [findings.md](02-findings.md) F-6.

Source: `packages/core/src/plugins/parsers/*`, `languages/configs/*`, `languages/frameworks/*`.

---

## 1. The 12 non-code parsers and what they emit

| Parser | Input | Cortex nodes | Cortex edges | Priority |
|--------|-------|--------------|--------------|----------|
| `sql-parser.ts` | `.sql` (DDL/DML, migrations) | `table`, `schema` | `defines_schema`, `migrates`, `reads_from`/`writes_to` (from queries) | **P1** |
| `terraform-parser.ts` | `.tf` | `resource`, `service` | `provisions`, `depends_on` | **P1** |
| `protobuf-parser.ts` | `.proto` | `schema`, `endpoint`, `service` | `defines_schema`, `routes` | **P1** |
| `graphql-parser.ts` | `.graphql`/`.gql` | `schema`, `endpoint` | `defines_schema`, `routes` | **P2** |
| `dockerfile-parser.ts` | `Dockerfile` | `config`, `service` | `deploys`, `depends_on` | **P2** |
| `yaml-parser.ts` | `.yml`/`.yaml` (k8s, compose, CI) | `config`, `service`, `pipeline` | `configures`, `deploys`, `triggers` | P2 |
| `toml-parser.ts` | `.toml` (Cargo, pyproject) | `config` | `configures`, `depends_on` | P2 |
| `json-parser.ts` | `.json` (package.json, tsconfig) | `config` | `configures`, `depends_on` | P3 |
| `env-parser.ts` | `.env` | `config` | `configures` | P3 |
| `makefile-parser.ts` | `Makefile` | `pipeline` | `triggers`, `depends_on` | P3 |
| `shell-parser.ts` | `.sh`/`.bash` | `pipeline` | `triggers` | P3 |
| `markdown-parser.ts` | `.md` | `document`, `article` | `documents`, `cites`, wikilink → `related` | P2 (KB lane) |

Priority = recommended Cortex implementation order, weighted by HiveLLM-ecosystem value (SQL +
Terraform + protobuf first: tables, infra, and service contracts answer "what touches X").

---

## 2. Language-config breadth (the `configs/*` list = a coverage checklist)

UA enumerates ~40 file types in `languages/configs/`. Cortex doesn't need all, but the list is a
ready-made coverage backlog:

- **Code (tree-sitter extractors exist):** typescript, javascript, python, rust, go, java, cpp, c,
  csharp, php, ruby, kotlin, swift, lua
- **Infra/CI:** dockerfile, docker-compose, kubernetes, terraform, github-actions, jenkinsfile,
  makefile, shell, powershell
- **Data/schema:** sql, protobuf, graphql, json-schema, openapi, csv
- **Config/markup:** yaml, toml, env, json-config, xml, html, css
- **Docs:** markdown, restructuredtext, plaintext

→ For Cortex, the **infra + data/schema** rows are the differentiator; code is already covered by
Cortex's existing extraction; config/markup is cheap incremental coverage.

---

## 3. Framework registry (route/endpoint awareness)

`languages/frameworks/*` lets the extractor recognize framework idioms and emit `endpoint`/`routes`
edges the raw AST wouldn't reveal:

| Framework | Emits |
|-----------|-------|
| express, gin | HTTP route → `endpoint` + `routes` |
| nextjs | file-based routes → `endpoint` |
| django, flask, fastapi, rails, spring | route decorators/URLconf → `endpoint` + `routes` |
| react, vue | component tree → `contains` |

**Cortex relevance:** route detection makes "which service exposes endpoint X / who calls it"
answerable. Port as a small framework-detection layer on top of the language extractor; prioritize
the frameworks actually used across HiveLLM repos.

---

## 4. Cortex port design

```text
trait Parser {
  fn matches(path: &Path) -> bool;            // by extension / filename
  fn parse(content: &str, path: &Path) -> ParseResult;  // -> facts (nodes+edges, deterministic)
}
// registry: Vec<Box<dyn Parser>>, first match wins; falls back to tree-sitter for code.
```

Each parser is deterministic (Phase-1 of the [extraction-contract.md](05-extraction-contract.md)); its
output goes through the same reconciliation gate before LLM annotation. Parsers are independently
testable with golden files (UA does exactly this in `__tests__/parsers.test.ts`).

**Build order (one parser per Rulebook sub-task, per incremental-implementation rule):**
P1: sql → terraform → protobuf. P2: graphql → dockerfile → yaml → markdown. P3: the rest as needed.

Each sub-task gate: golden-file test asserting exact node/edge output for a fixture file.
