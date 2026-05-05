# TmlDocs — Operational

## Build Process

### Prerequisites
- Node.js 20.x or Bun 1.x (Bun recommended)
- PostgreSQL 15+ (or Neon account for dev)

### Local Setup

```bash
# Clone and install
git clone https://github.com/hivellm/tml-docs
cd tml-docs
bun install

# Setup environment
cp .env.example .env.local
# Edit .env.local with GitHub OAuth credentials, DATABASE_URL

# Run dev server
bun run dev  # Starts all apps at localhost:5173 (site), localhost:5174 (registry)
```

### Build Commands

| Command | Purpose |
|---------|---------|
| `bun run build` | Build all apps/packages (production) |
| `bun run dev` | Start dev servers (HMR enabled) |
| `bun run type-check` | Run tsc --noEmit (all workspaces) |
| `bun run lint` | Run ESLint (zero warnings required) |
| `bun test` | Run Vitest (all workspaces) |
| `bun run test:coverage` | Vitest with coverage (≥95% required) |

## Deployment

### Frontend (Vercel)

**apps/site** and **apps/registry** connect to Vercel via GitHub.

**Process**:
1. Push to `main` branch
2. Vercel runs `bun run build`, deploys static assets to CDN
3. Preview deploys on every PR
4. Environment vars (GitHub OAuth client_id, API endpoint) set in Vercel dashboard

**Endpoints**:
- tml-lang.org (site)
- package.tml-lang.org (registry frontend)

### Backend API

**packages/api** deploys as:

**Option A: Cloudflare Workers**
- Wrangler CLI deploys via `wrangler publish`
- Global edge network
- Connects to Neon PostgreSQL (outbound via Cloudflare tunnel)

**Option B: Fly.io**
- Dockerfile-based deployment
- `flyctl deploy`
- Simpler PostgreSQL connectivity

**Both**:
- Environment: GitHub OAuth secret, Neon CONNECTION_STRING, GITHUB_TOKEN
- Database migrations run on deploy (Drizzle auto-migration)

### Database (Neon PostgreSQL)

- **Provisioning**: Create Neon project, note CONNECTION_STRING
- **Migrations**: Drizzle manages schema (seed scripts for advisories)
- **Backups**: Neon auto-snapshots every 24h, point-in-time recovery available
- **Monitoring**: Neon dashboard for query performance, disk usage

## Monitoring and Observability

- **Error tracking**: Sentry (JS errors from frontend + backend)
- **Analytics**: Plausible or Umami (privacy-friendly, no cookies)
- **Database**: Neon dashboard (slow queries, replication lag)
- **Log aggregation**: Vercel + Fly.io built-in logs

## CI/CD Pipeline (GitHub Actions)

**On every push to PR or main**:
1. Lint (ESLint) → fail if warnings
2. Type-check (tsc --noEmit) → fail if errors
3. Build (bun run build) → fail if build errors
4. Test (Vitest) → fail if tests don't pass, coverage < 95%

**On merge to main**:
1. Build succeeds (above)
2. Vercel auto-deploys frontend
3. Wrangler/Fly.io publishes backend
4. Migrations run (Drizzle)

## Maintenance Tasks

| Task | Frequency | Owner |
|------|-----------|-------|
| Advisory database updates | Weekly | Automation (cron job) |
| Package audits (re-run all) | Daily | Automation (background worker) |
| Install stats rollup | Daily | Automation (SQL aggregate) |
| Certificate renewal | N/A (auto via Vercel/Cloudflare) | N/A |
| Database optimization (VACUUM) | Monthly | Neon (auto) |
| Dependency updates | Bi-weekly | Renovate/Dependabot |

## Troubleshooting

**Build fails in CI but passes locally**:
- Check Node.js/Bun version match
- Run `bun install --frozen-lockfile` (ensure exact deps)

**Database connectivity errors**:
- Verify CONNECTION_STRING in .env.local
- Check Neon dashboard for active connections
- Test: `psql $CONNECTION_STRING -c "SELECT 1"`

**Registry API 5xx errors**:
- Check Cloudflare/Fly.io logs
- Verify GitHub OAuth credentials are current
- Run migrations: `drizzle-kit push`
