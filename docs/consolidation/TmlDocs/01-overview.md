# TmlDocs — Overview

## Purpose

TmlDocs is a **documentation and package registry platform** for the TML language (Templating Markup Language, LLM-optimized). It serves two primary audiences:

1. **Developers learning TML** — through landing page, comprehensive documentation, guides, and blog
2. **Package authors and consumers** — through a searchable package registry enabling publishing and discovery

## Project Role

- **Site**: tml-lang.org — marketing site + language documentation
- **Registry**: package.tml-lang.org — index-based package registry (Git-backed, index-only, no tarballs)
- **Foundation**: bootstraps the documentation and registry ecosystem for the TML language
- **Part of HiveLLM**: integrates with TML compiler (upstream) and Cortex (downstream indexing)

## Stack

### Frontend
- **Monorepo**: Turborepo (orchestration) + Bun (package manager)
- **Framework**: React + TypeScript 5.x, Vite bundler
- **Styling**: Tailwind CSS 4.x
- **Testing**: Vitest 2.x + React Testing Library

### Backend
- **API**: Hono framework running on Bun/Node.js or Cloudflare Workers
- **Database**: PostgreSQL (Neon serverless) — metadata only
- **ORM**: Drizzle
- **Auth**: GitHub OAuth

### Infrastructure
- **Frontend Hosting**: Vercel (tml-lang.org, package.tml-lang.org)
- **API Hosting**: Cloudflare Workers or Fly.io (stateless)
- **Database**: Neon PostgreSQL (serverless)

## Key Design Principle

**Index-only registry** (like Go modules, Deno) — packages live in Git repos, registry stores metadata only. No tarballs, no file storage, no R2 object buckets. Installation fetches source directly from Git tags.
