# TmlDocs — Data and Storage

## Content Layout

### Documentation Content (apps/site/content/)

```
content/
├── getting-started/
│   ├── installation.md
│   ├── hello-world.md
│   └── first-project.md
├── language-reference/
│   ├── types.md
│   ├── functions.md
│   ├── generics.md
│   └── behaviors.md
├── standard-library/
│   └── (auto-generated from MCP docs)
├── guides/
│   ├── error-handling.md
│   ├── ffi.md
│   ├── testing.md
│   └── modules.md
└── blog/
    ├── 2026-03-28-introducing-tml.md
    └── ...
```

**Format**: Markdown/MDX (GitHub-flavored, remark-gfm enabled)

**Rendering**: @mdx-js/rollup → React components with Shiki syntax highlighting

## Database Schema (PostgreSQL — Index-Only)

### Core Tables

**users**
- id (UUID, PK)
- username, email, avatar_url
- github_id (OAuth)
- created_at, updated_at

**packages**
- id (UUID, PK)
- name (UNIQUE)
- repository (https://github.com/...; UNIQUE)
- description, readme (cached), homepage
- created_at, updated_at

**versions**
- id (UUID, PK)
- package_id (FK)
- version (semver)
- git_tag, git_commit_sha
- path_in_repo (for monorepo)
- published_at, yanked_at

**dependencies**
- id (UUID, PK)
- version_id (FK)
- package_name, version_spec
- is_dev_dependency

**tokens**
- id (UUID, PK)
- user_id (FK)
- token_value (hashed)
- name, scopes (array: [publish-new, publish-update, yank])
- created_at, last_used_at

**audit_results**
- id (UUID, PK)
- version_id (FK)
- audit_type (structural, security, quality)
- status (pass, warn, fail)
- details (JSON)
- audited_at

**advisories**
- id (UUID, PK)
- package_name, version_range
- severity (critical, high, medium, low)
- description, cve_id, url
- published_at

**install_stats**
- id (UUID, PK)
- version_id (FK)
- date (for daily aggregation)
- install_count

### No File Storage

- **No tarballs**: packages live in Git repos
- **No object storage (R2, S3)**: registry is metadata-only
- **No source code blobs**: everything references Git coordinates
- **README caching**: fetched from GitHub at publish time, stored as text

## Data Consistency

**GitHub API Sync**:
- On publish: fetch README/LICENSE at exact git tag
- On audit: re-validate repo accessibility
- Daily: re-fetch README for popular packages (staleness)

**Advisory Updates**:
- Ingest from upstream feeds (NVD, GitHub Security Advisory Database)
- Re-audit all versions daily against latest advisories
- Notify users of new vulnerabilities
