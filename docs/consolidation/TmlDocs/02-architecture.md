# TmlDocs — Architecture

## Monorepo Structure (Turborepo)

```
tml-docs/
├── apps/
│   ├── site/          — tml-lang.org (Vite + React SPA)
│   └── registry/      — package.tml-lang.org (Vite + React SPA)
├── packages/
│   ├── ui/            — Shared React components (Tailwind)
│   ├── api/           — Hono backend (TypeScript, Drizzle)
│   ├── config/        — Shared tsconfig, eslint, tailwind
│   └── tml-highlight/ — TML syntax grammar for Shiki
└── turbo.json         — Monorepo configuration
```

## Site Generator Architecture

### Landing Page (tml-lang.org)

**Pages**:
- Homepage hero → features → benchmarks → call-to-action
- Docs system (MDX-based)
- Blog (MDX posts with date, author, tags)

**Content Pipeline**:
1. Markdown/MDX files in `apps/site/content/`
2. @mdx-js/rollup compiles to React components
3. rehype-shiki applies syntax highlighting (includes TML grammar)
4. remark-gfm adds GitHub-flavored markdown support
5. Vite bundles into optimized SPA

### Package Registry Frontend (package.tml-lang.org)

**Pages**:
- Package search (full-text, filters, results grid)
- Package detail (README, versions, dependencies, install command)
- Version detail (tml.toml, dependency tree, package size)
- User profile (avatar, published packages, stats)
- Login (GitHub OAuth redirect)
- Settings (profile, API token management)

### Registry Backend API

**Model**: Index-only, stateless

**Core Endpoints**:
- `POST /api/publish` — record package version (repo, tag, metadata)
- `GET /api/packages/:name` — retrieve package metadata
- `GET /api/search` — full-text search packages
- `POST /api/audit` — check dependencies against advisories
- `GET /api/users/:username` — user profile + packages

**Background Worker**:
- Audit checks after each publish (structural, security, quality)
- Daily re-audit against new advisories
- Install stat aggregation

## TML Syntax Highlighting

**File**: `packages/tml-highlight/`

- **TextMate grammar** for TML syntax
- Registered as a **Shiki language**
- Used in code blocks across docs and registry
- Supports line highlighting, tabs, copy buttons

## Testing Architecture

- **Vitest** configured at root (shared test runner)
- **Per-workspace** vitest configs for package-specific tests
- **React Testing Library** for component tests
- **Supertest** or equivalent for API integration tests
- Minimum **95% coverage** target

## CI/CD

**GitHub Actions**:
- Lint (ESLint)
- Type-check (tsc --noEmit)
- Test (Vitest)
- Build (each app/package)
- Deploy on merge to main (Vercel front-end, Workers/Fly.io backend)
