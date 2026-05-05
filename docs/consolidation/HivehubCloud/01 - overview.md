# HiveHub.Cloud: Project Overview

**Status**: SaaS platform in active development (v1.0.0)  
**Purpose**: Cloud control plane providing Hive services (Vectorizer, Nexus, Synap, LessTokens) in SaaS format  
**Maturity**: Foundation architecture and specifications complete; core features (auth, subscriptions, projects) implemented; integration work ongoing

---

## What It Is

HiveHub.Cloud is a **multi-tenant SaaS platform** that aggregates four Hive services behind a unified cloud control plane. Users authenticate, subscribe to plans, organize projects, and access integrated Hive service capabilities via REST API + unified SDK.

**Key tagline**: "One account, all Hive services. Pay once. Use anywhere."

---

## Why It Matters to Cortex

1. **User/Subscription Context**: Cortex ingests data *for* HiveHub.Cloud users; need to understand user quotas, plan limits, and multi-tenancy model.
2. **Orchestration Hub**: HiveHub.Cloud orchestrates file upload → Transmutation → Vectorizer indexing → Nexus graph sync. Cortex must track this pipeline.
3. **Service API Gateway**: HiveHub.Cloud exposes `/api/internal/{service}/...` endpoints that Vectorizer/Nexus/Synap call for quota enforcement and data segregation.
4. **MCP Server Generation**: Users can generate authenticated MCP servers for each Hive service via HiveHub.Cloud dashboard. Cortex may need to track these.

---

## Core Stack

| Layer | Technology | Notes |
|-------|-----------|-------|
| **Backend API** | Rust (Axum, Tokio) + PostgreSQL 16+ | Single binary; high-performance; 73+ passing tests; 95%+ coverage |
| **Frontend** | React 18+ (TypeScript) + Vite | Dashboard for UI; planned; not yet started |
| **Auth** | JWT + OAuth (Google, GitHub) | Native provider integration; bcrypt password hashing |
| **Payments** | Stripe + PayPal | Webhook-based; recurring billing planned |
| **Integrations** | Vectorizer, Nexus, Synap, LessTokens | Via internal REST API + Rust SDK in services |
| **Deployment** | Docker; PostgreSQL required | Environment-based config; scalable horizontally |

---

## Subscription Tiers

| Plan | Storage | Documents | Collections | Price |
|------|---------|-----------|-------------|-------|
| **Free** | 50MB | 1,000 | 1 (10k vectors) | Free |
| **Pro** | 500MB | 10,000 | Multiple | ~$20/mo |
| **Enterprise** | 5GB | 100,000 | Unlimited | ~$200/mo |

Storage and limits are **shared across all user projects**.

---

## Multi-Tenancy Model

- **User account** → **Projects** (multiple per user) → **Collections** (Vectorizer) + **Graph** (Nexus) + **Key-value** (Synap)
- Data isolation enforced at collection/graph/KV namespace level
- Quotas tracked per user (shared across projects)
- Service API keys used by Hive services to query user quotas/data

---

## Orchestrated Workflows

1. **File Upload Pipeline**:
   - User uploads file via dashboard/API
   - Transmutation service converts to Markdown
   - Markdown chunked and embedded
   - Vectors stored in user's Vectorizer collection
   - File metadata tracked in PostgreSQL

2. **MCP Server Creation**:
   - User creates authenticated MCP server in dashboard
   - HiveHub.Cloud generates service-specific API key
   - Returns MCP server endpoint + auth token
   - User can configure Vectorizer/Nexus/Synap as MCP context providers

---

## Open Questions / Gaps

1. **GraphQL API**: Planned but not yet specified; REST endpoints only for now.
2. **Telemetry/Analytics**: Dashboard analytics planned but not yet implemented.
3. **Recurring Billing**: Payment infrastructure in place; recurring billing spec pending.
4. **Rate Limiting**: Not yet implemented; future enhancement.
5. **Multi-region Deployment**: Currently assumes single-region PostgreSQL.

---

## Repository & Access

- **Repo**: github.com/hivellm/hivehub-cloud
- **License**: Apache 2.0
- **Maintained by**: HiveHub.Cloud Team
- **Status**: 🚧 In Development (specs 100%, implementation ~40%)
