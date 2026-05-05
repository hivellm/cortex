# TmlDocs — Decisions and Rationale

## Index-Only Package Registry (Not Storage-Based)

**Decision**: Packages live in Git repos; registry stores metadata only (like Go modules, Deno, unlike npm, crates.io)

**Rationale**:
- **Simplicity**: No tarball upload, no file storage system, no R2 or S3 needed
- **Decentralization**: Authors keep source code on their own Git infrastructure
- **Trust**: Installation from source, not distributed binaries
- **Cost**: No object storage costs (major Cortex/HiveLLM advantage)
- **Git-native**: Aligns with TML developer workflow (Git-first)

**Trade-offs**:
- No pre-built artifacts (compile on install) — acceptable for TML's use case
- More network traffic on install (clone repos instead of tarball) — mitigated by sparse-clone
- Monorepo discovery requires path_in_repo field — implemented

## GitHub OAuth as Primary Auth

**Decision**: Only GitHub OAuth at launch (no email/password, no Google/Microsoft)

**Rationale**:
- **Developer audience**: 95%+ of TML developers have GitHub accounts
- **Trust**: GitHub's audit trail valuable for open source
- **Simplicity**: Reduces auth complexity, no email verification, no password resets
- **TOML integration**: Credentials stored in ~/.tml/ alongside TML config

**Trade-offs**:
- Excludes non-GitHub developers (minimal impact for core launch)
- Tied to GitHub service availability

## Turborepo + Bun Monorepo

**Decision**: Use Turborepo 2.x for orchestration, Bun 1.x for package management

**Rationale**:
- **Turborepo**: Fast, battle-tested for multi-app monorepos (Vercel uses it internally)
- **Bun**: Fast task runner, integrated test runner (Vitest), zero-config feel
- **Shared config**: packages/config/ eliminates tsconfig/eslint duplication
- **Workspace sharing**: node_modules deduped, fast installs

**Trade-offs**:
- Bun still emerging (but HiveLLM uses it across projects)
- Turborepo learning curve (mitigated by well-documented workflows)

## Tailwind CSS + Shadcn/ui Pattern

**Decision**: Style with Tailwind CSS 4.x, ship reusable component library in packages/ui/

**Rationale**:
- **Consistency**: Same design system across landing page and registry
- **Performance**: Tailwind's JIT compiler produces minimal CSS
- **Developer velocity**: Utility-first rapid iteration
- **Component reuse**: Button, Card, Dialog, etc. shared across apps

**Trade-offs**:
- Tailwind's class verbosity (mitigated by component abstractions)
- Learning curve for team (offset by online community)

## Vercel for Frontend, Cloudflare/Fly.io for Backend

**Decision**: Deploy React SPAs to Vercel, Hono API to Cloudflare Workers or Fly.io

**Rationale**:
- **Vercel**: Native Next.js support (though we use Vite); excellent SPA support; preview deploys
- **Cloudflare Workers**: Global edge, serverless, no cold starts (perfect for stateless API)
- **Fly.io** (alternative): Better for stateful services, simpler Hono integration, cheaper
- **Database**: Neon PostgreSQL (serverless, replicated, auto-scaling)

**Trade-offs**:
- Workers: Fewer GBs memory per request (mitigated by stateless design)
- Fly.io: Smaller global footprint than Cloudflare (acceptable for developer audience)

## MDX + Shiki + rehype-gfm Content Pipeline

**Decision**: Markdown/MDX with syntax highlighting via Shiki (TML grammar included)

**Rationale**:
- **MDX**: Embed React components in docs (interactive examples, callouts)
- **Shiki**: Fast, bundled syntax highlighting (TML + 100+ languages)
- **rehype-gfm**: GitHub-flavored markdown (tables, strikethrough, etc.)
- **Pagefind**: Static search (fast, privacy-friendly, no external deps)

**Trade-offs**:
- No dynamic docs generation (static build required) — acceptable, docs don't change constantly
- Shiki bundle size (mitigated by code splitting)

## PostgreSQL-Only (No Cache Layers)

**Decision**: Neon PostgreSQL as single source of truth; no Redis/Memcached at launch

**Rationale**:
- **Simplicity**: Fewer moving parts, easier deployment, lower ops burden
- **Sufficient performance**: index-only registry doesn't require heavy caching
- **Neon replication**: Built-in HA, point-in-time recovery
- **Can add caching later**: Cloudflare KV or Redis if needed

**Trade-offs**:
- Higher database load (mitigated by query optimization, indexes)
- No distributed cache for advisory lists (re-computed per request) — acceptable for small package count at launch

## 95% Test Coverage Minimum

**Decision**: All new code must achieve ≥95% coverage (tests, 100% passing)

**Rationale**:
- **Registry reliability**: Metadata corruption costly for package ecosystem
- **Consistency**: All HiveLLM projects enforce 95%+ coverage
- **Regression prevention**: High coverage catches breaking changes early

**Trade-offs**:
- Takes longer to implement (offset by preventing bugs)
- Requires discipline writing testable code
