# Understand-Anything — Full File Inventory

Every file in `Lum1104/Understand-Anything@main`, enumerated and annotated for Cortex relevance.
Source: GitHub git-tree API (`recursive=1`).

**Relevance legend:** ⭐ high (study/port) · ◐ medium · · low/none · 🧪 test · 📄 doc/asset.

---

## Root

| File | Rel | Note |
|------|-----|------|
| `README.md` | ⭐ | Feature list + "Under the Hood" pipeline |
| `CLAUDE.md` | ◐ | Their agent instructions |
| `LICENSE` | ⭐ | **Check before lifting code** |
| `CONTRIBUTING.md` / `CODE_OF_CONDUCT.md` / `SECURITY.md` | · | governance |
| `package.json` / `pnpm-lock.yaml` / `pnpm-workspace.yaml` | · | monorepo wiring |
| `tsconfig.json` / `eslint.config.mjs` | · | toolchain |
| `.npmrc` / `.gitignore` | · | |
| `install.sh` / `install.ps1` | ◐ | install/bootstrap pattern |

## Plugin manifests

| File | Rel | Note |
|------|-----|------|
| `.claude-plugin/marketplace.json` | · | Claude Code marketplace entry |
| `.claude-plugin/plugin.json` | ◐ | plugin manifest shape |
| `.copilot-plugin/plugin.json` | · | Copilot manifest |
| `.cursor-plugin/plugin.json` | · | Cursor manifest |
| `understand-anything-plugin/.claude-plugin/plugin.json` | · | inner manifest |

## i18n READMEs / assets / GitHub meta

| Path | Rel | Note |
|------|-----|------|
| `READMEs/README.{es-ES,ja-JP,ko-KR,ru-RU,tr-TR,zh-CN,zh-TW}.md` | 📄 | translations |
| `assets/hero.png`, `assets/overview.png` | 📄 | images |
| `.github/workflows/ci.yml` | ◐ | their CI |
| `.github/workflows/deploy-homepage.yml` | · | pages deploy |
| `.github/ISSUE_TEMPLATE/*`, `PULL_REQUEST_TEMPLATE.md`, `FUNDING.yml` | · | repo meta |

---

## ⭐ Agents — `understand-anything-plugin/agents/` (the 9-agent pipeline)

| File | Rel | Role |
|------|-----|------|
| `project-scanner.md` | ⭐ | Stage 1: discover files, langs, frameworks |
| `file-analyzer.md` | ⭐ | Two-phase extract→semantic; anti-hallucination contract (F-3) |
| `architecture-analyzer.md` | ⭐ | layer detection |
| `domain-analyzer.md` | ◐ | business domain/flow/step extraction |
| `tour-builder.md` | ◐ | dependency-ordered guided tour (F-8) |
| `graph-reviewer.md` | ⭐ | referential-integrity validation pass |
| `assemble-reviewer.md` | ◐ | multi-batch assembly review |
| `article-analyzer.md` | ⭐ | Karpathy KB: wikilinks → claim/entity/topic (F-5) |
| `knowledge-graph-guide.md` | ◐ | how the model queries the graph |

## ⭐ Hooks — `understand-anything-plugin/hooks/`

| File | Rel | Note |
|------|-----|------|
| `hooks.json` | ⭐ | PostToolUse(git commit)+SessionStart auto-update (F-7) |
| `auto-update-prompt.md` | ⭐ | the self-executing incremental-update directive |

---

## ⭐⭐ Core library — `understand-anything-plugin/packages/core/src/`

### Top-level modules

| File | Rel | What |
|------|-----|------|
| `types.ts` | ⭐ | **Node/edge ontology** (21 node, 35 edge types) (F-4) |
| `schema.ts` | ⭐ | graph schema validation |
| `index.ts` | ◐ | public surface |
| `search.ts` | ◐ | Fuse.js fuzzy (reject for Cortex, F-?) |
| `embedding-search.ts` | ◐ | in-memory cosine (reject; Cortex has HNSW) |
| `fingerprint.ts` | ⭐ | file fingerprint (change detection) (F-1) |
| `staleness.ts` | ⭐ | git-hash staleness + `mergeGraphUpdate` (F-1) |
| `change-classifier.ts` | ⭐ | SKIP/PARTIAL/ARCH/FULL tiers (F-2) |
| `ignore-filter.ts` | ◐ | `.understandignore` honoring |
| `ignore-generator.ts` | ◐ | auto-generate ignore file |

### `analyzer/`

| File | Rel | What |
|------|-----|------|
| `graph-builder.ts` | ⭐ | assembles nodes/edges into the graph |
| `llm-analyzer.ts` | ⭐ | Phase-2 semantic LLM pass |
| `layer-detector.ts` | ◐ | architectural-layer inference |
| `normalize-graph.ts` | ⭐ | dedupe/normalize edges+nodes (portable rules) |
| `tour-generator.ts` | ◐ | builds `tour: TourStep[]` |
| `language-lesson.ts` | · | "12 programming patterns" teaching content |

### `plugins/extractors/` (deterministic, tree-sitter, per language)

| File | Rel |
|------|-----|
| `base-extractor.ts`, `types.ts`, `index.ts` | ⭐ extractor trait/registry — port shape to Rust |
| `typescript-extractor.ts` | ◐ |
| `python-extractor.ts` | ◐ |
| `rust-extractor.ts` | ⭐ (Cortex is Rust — reference) |
| `go-extractor.ts`, `java-extractor.ts`, `cpp-extractor.ts`, `csharp-extractor.ts`, `php-extractor.ts`, `ruby-extractor.ts` | ◐ |

### `plugins/parsers/` (deterministic, non-code — F-6)

| File | Rel | Emits |
|------|-----|-------|
| `index.ts` | ⭐ | parser registry |
| `sql-parser.ts` | ⭐ | `table`/`schema` + `defines_schema`/`migrates` |
| `terraform-parser.ts` | ⭐ | `resource` + `provisions` |
| `protobuf-parser.ts` | ⭐ | `schema`/`endpoint` + `defines_schema` |
| `graphql-parser.ts` | ⭐ | `schema`/`endpoint` + `routes` |
| `dockerfile-parser.ts` | ⭐ | `config`/`service` + `deploys` |
| `yaml-parser.ts`, `toml-parser.ts`, `json-parser.ts`, `env-parser.ts` | ◐ | `config` |
| `makefile-parser.ts`, `shell-parser.ts` | ◐ | `pipeline`/`triggers` |
| `markdown-parser.ts` | ◐ | `document` + wikilink edges |

### `plugins/` infra

| File | Rel | What |
|------|-----|------|
| `tree-sitter-plugin.ts` | ⭐ | tree-sitter driver |
| `registry.ts` | ⭐ | plugin registry (extractors+parsers) |
| `discovery.ts` | ◐ | plugin auto-discovery |

### `languages/` (config-driven language + framework registry)

| Path | Rel | What |
|------|-----|------|
| `language-registry.ts`, `index.ts`, `types.ts` | ◐ | language registry |
| `configs/*.ts` (40 files: `rust.ts`, `python.ts`, `typescript.ts`, `go.ts`, `sql.ts`, `terraform.ts`, `protobuf.ts`, `graphql.ts`, `dockerfile.ts`, `kubernetes.ts`, `openapi.ts`, `json-schema.ts`, `github-actions.ts`, `jenkinsfile.ts`, `markdown.ts`, … ) | ◐ | per-language extract config — **the breadth of file types is the value**; mine the list for Cortex parser coverage |
| `framework-registry.ts` | ◐ | framework registry |
| `frameworks/*.ts` (`react`, `vue`, `nextjs`, `express`, `django`, `flask`, `fastapi`, `rails`, `spring`, `gin`, `index`) | ◐ | framework-aware extraction (route/endpoint detection) |

### `persistence/`

| File | Rel | What |
|------|-----|------|
| `persistence/index.ts` | ◐ | read/write `knowledge-graph.json` + `meta.json` (reject storage, study meta shape) |

---

## 🧪 Tests — study as executable spec

| Path | Rel | Pins behavior of |
|------|-----|------------------|
| `packages/core/src/__tests__/change-classifier.test.ts` | ⭐ | tier thresholds (F-2) |
| `…/staleness.test.ts` | ⭐ | incremental merge (F-1) |
| `…/fingerprint.test.ts` | ⭐ | change detection (F-1) |
| `…/schema.test.ts`, `…/normalize-graph.test.ts` | ⭐ | graph integrity/normalize |
| `…/parsers.test.ts` | ◐ | parser outputs (F-6) |
| `…/search.test.ts`, `…/embedding-search.test.ts` | ◐ | (reject features) |
| `…/domain-*.test.ts` (`normalize`,`persistence`,`types`) | ◐ | domain layer |
| `…/layer-detector.test.ts`, `…/tour-generator.test.ts` | ◐ | layers/tours |
| `…/language-registry.test.ts`, `…/framework-registry.test.ts`, `…/language-lesson.test.ts` | · | registries |
| `…/ignore-filter.test.ts`, `…/ignore-generator.test.ts`, `…/plugin-discovery.test.ts`, `…/plugin-registry.test.ts` | · | infra |
| `plugins/extractors/__tests__/*-extractor.test.ts` (cpp, csharp, go, java, php, python, ruby, rust) | ◐ | extractor golden files |
| `analyzer/graph-builder.test.ts`, `analyzer/llm-analyzer.test.ts` | ⭐ | build/semantic passes |
| `persistence/persistence.test.ts`, `types.test.ts`, `plugins/tree-sitter-plugin.test.ts` | ◐ | |
| `tests/skill/understand/test_*.{mjs,py}` + `fixtures/*.json` | ◐ | end-to-end skill tests (batching, import-map, merge, scan) — fixtures show real graph JSON shape |

---

## · Dashboard — `understand-anything-plugin/packages/dashboard/` (NOT a Cortex target)

React + reactflow viewer. Cortex has its own GUI; skim only for UX ideas.

- `src/App.tsx`, `src/main.tsx`, `index.html`, `index.css`
- `components/` (50+): `GraphView`, `DomainGraphView`, `KnowledgeGraphView`, `CodeViewer`,
  `DiffToggle`, `PathFinderModal`, `SearchBar`, `FilterPanel`, `PersonaSelector`, `LearnPanel`,
  `NodeInfo`, `Breadcrumb`, `*ClusterNode`, mobile `*` … → ◐ only `DiffToggle`/`PathFinderModal`/
  `KnowledgeGraphView` interesting for Cortex GUI parity (diff-impact F-9, path queries).
- `contexts/I18nContext.tsx`, `hooks/useIsMobile.ts`, `hooks/useKeyboardShortcuts.ts`
- `locales/{en,ja,ko,ru,zh,zh-TW}.ts`
- `scripts/benchmark-aggregations.mjs`, `scripts/benchmark-layout.mjs` → ◐ graph-scaling benchmarks
- `public/knowledge-graph.json` → ⭐ **a real serialized graph sample — read it for the concrete JSON shape**

## 📄 Homepage — `homepage/` (marketing site, Astro)

`src/components/*.astro` (Hero, Features, Install, Showcase, Problem…), `layouts/Layout.astro`,
`pages/index.astro`, `styles/global.css`, fonts/images. → · none for Cortex.

## ⭐ Design docs — `docs/superpowers/` (their own spec/plan history — high-signal reading)

**Specs** (`docs/superpowers/specs/`):
- `2026-03-14-understand-anything-design.md` ⭐ — core design
- `2026-03-21-language-agnostic-design.md` ⭐ — how they generalized past one language
- `2026-04-01-business-domain-knowledge-design.md` ◐
- `2026-04-09-understand-knowledge-design.md` ⭐ — Karpathy KB graph (F-5)
- `2026-04-10-understandignore-design.md` ◐
- `2026-05-03-graph-layout-scaling-design.md` ◐ — scaling large graphs
- `2026-05-24-semantic-batching-and-output-chunking-design.md` ⭐ — batching/chunking LLM output (relevant to Cortex worker batching)
- `2026-06-03-language-auto-detection-design.md` ◐
- (+ homepage/theme/token-reduction/multi-platform designs → ·)

**Plans** (`docs/superpowers/plans/`): 20 dated impl plans mirroring the specs — ◐ read alongside
each spec for the "how it was actually built" narrative. Notably
`2026-05-24-semantic-batching-and-output-chunking-impl.md`,
`2026-04-15-language-extractors-impl.md`, `2026-03-27-token-reduction-impl.md`.

## · Misc scripts

- `scripts/generate-large-graph.mjs` → ◐ synthetic large-graph generator (load-testing idea)

---

## Reading order for the Cortex team (highest signal first)

1. `packages/core/src/types.ts` — the ontology → [ontology-mapping.md](03-ontology-mapping.md)
2. `staleness.ts` + `change-classifier.ts` + `fingerprint.ts` — incremental → [incremental-patching.md](04-incremental-patching.md)
3. `agents/file-analyzer.md` + `analyzer/llm-analyzer.ts` — extraction contract → [extraction-contract.md](05-extraction-contract.md)
4. `plugins/parsers/*` + `__tests__/parsers.test.ts` — non-code coverage → [parsers.md](06-parsers.md)
5. `agents/article-analyzer.md` + `docs/superpowers/specs/2026-04-09-understand-knowledge-design.md` — claim graph
6. `dashboard/public/knowledge-graph.json` — a real serialized graph
7. `docs/superpowers/specs/2026-05-24-…batching…-design.md` — LLM output batching/chunking
