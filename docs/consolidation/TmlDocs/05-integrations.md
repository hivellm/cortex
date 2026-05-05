# TmlDocs — Integrations

## TML Compiler (Upstream)

**Relationship**: TmlDocs documents and indexes the TML compiler

**Integration Points**:
- **Documentation**: stdlib API docs auto-generated from MCP (Magic Capability Protocol) documentation
- **Syntax Highlighting**: TmlDocs ships TML TextMate grammar used by TML editor extensions
- **CLI Integration**: TML compiler's `tml publish`, `tml login`, `tml audit` commands target TmlDocs registry API
- **Version Constraints**: tml.toml manifest specifies `tml-version = ">=0.3.0"` requirements

**Data Flow**:
```
TML compiler stdlib MCP → TmlDocs registry (docs auto-gen) → package.tml-lang.org
TML compiler CLI → registry API (publish, audit, search)
```

## TmlTextmate (Related Project)

**Purpose**: Editor extension (VS Code, Sublime) providing TML syntax highlighting

**Integration**:
- Uses the **same TextMate grammar** as TmlDocs (`packages/tml-highlight/`)
- Single source of truth: grammar defined in TmlDocs, consumed by both editor extension and registry
- Updates to grammar propagate to both TmlDocs and editor via npm @hivellm/tml-highlight package

## GitHub OAuth

**Provider**: GitHub for authentication

**Flow**:
1. User clicks "Sign in with GitHub" on package.tml-lang.org
2. OAuth redirect to GitHub login
3. GitHub returns code to registry API
4. Registry creates/updates user record, generates API token
5. Token stored in ~/.tml/credentials.toml by CLI

**Scopes Requested**:
- `user:email` (read user email)
- `public_repo` (verify repo accessibility during publish)

## GitHub API (Publishing Workflow)

**Used by Registry API**:

On `tml publish repo-url tag-name`:
1. Validate repo is public and accessible
2. Verify git tag exists via GitHub API
3. Fetch README.md at exact tag
4. Fetch LICENSE at exact tag
5. Extract tml.toml from commit (monorepo path support)
6. Record version in PostgreSQL
7. Queue async audit job

**Endpoints**:
- `GET /repos/{owner}/{repo}` — verify repo exists
- `GET /repos/{owner}/{repo}/git/refs/tags/{tag}` — verify tag
- `GET /repos/{owner}/{repo}/contents/{path}@{tag}` — fetch files at tag
- `GET /repos/{owner}/{repo}/contents/README.md@{tag}` — fetch README (cached)

## Cortex (Downstream)

**Purpose**: HiveLLM's knowledge consolidation system (indexes all projects)

**TmlDocs → Cortex Integration**:
- **Stdlib documentation** from package.tml-lang.org/api rendered as indexed docs
- **Package registry metadata** (names, versions, README snippets) indexed for language-aware search
- **Blog posts** indexed as project announcements
- **Getting started guides** indexed as onboarding content

**Data Format**: Cortex ingests from TmlDocs via:
- HTTP scraping of package detail pages (HTML + JSON API endpoints)
- Structured exports (JSON API responses from registry)
- RSS feed for blog posts

## Codespaces / Remote Dev

**Not currently integrated** — TmlDocs is a web application, not a dev environment project. Local development uses Bun + Turborepo locally.
