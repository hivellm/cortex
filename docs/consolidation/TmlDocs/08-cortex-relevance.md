# TmlDocs — Cortex Relevance

## What Cortex Should Index from TmlDocs

Cortex (HiveLLM's knowledge consolidation system) should ingest the following from TmlDocs:

### 1. Standard Library Documentation

**Source**: tml-lang.org/docs/standard-library/ (auto-generated from TML compiler MCP)

**Content Type**: API reference documentation
- Function signatures, type definitions
- Parameter descriptions, return types
- Usage examples
- Deprecation notices, stability badges

**Value to Cortex**: Complete stdlib reference for code generation and prompt engineering

**Indexing Strategy**:
- Scrape or consume via API endpoint (package.tml-lang.org/api/stdlib)
- Update on each TML compiler release
- Tag with version (e.g., `tml:stdlib:0.3.0`)

### 2. Getting Started Guides

**Source**: tml-lang.org/docs/getting-started/

**Content Type**: Tutorial/onboarding
- Installation (by platform)
- Hello World program
- First project setup
- Common patterns

**Value to Cortex**: Onboarding content for new TML developers, prompt context

**Indexing Strategy**:
- Crawl markdown → convert to searchable embeddings
- Tag with `tutorial:tml:getting-started`

### 3. Language Reference (Types, Functions, Behaviors)

**Source**: tml-lang.org/docs/language-reference/

**Content Type**: Language specification
- Type system (primitives, generics, type aliases)
- Function declarations, closures
- Behaviors (async, error handling, traits)
- Operator precedence

**Value to Cortex**: Core language semantics for code generation tasks

**Indexing Strategy**:
- Index by section (e.g., `language-ref:generics`)
- Cross-link with stdlib docs

### 4. Package Registry Metadata

**Source**: package.tml-lang.org/api/packages

**Content Type**: Package metadata + README summaries
- Package name, version, description
- Repository link, git tag
- README excerpt (first 500 chars)
- Dependency graph
- Audit results (vulnerabilities, stability)

**Value to Cortex**: Package ecosystem awareness, dependency analysis

**Indexing Strategy**:
- Full-text index on package descriptions
- Build dependency graph in Cortex KG
- Tag vulnerable packages for alerts

**API Endpoint**:
```
GET /api/search?q=<query>&page=1
GET /api/packages/:name
GET /api/packages/:name/versions/:version
```

### 5. Blog Posts

**Source**: tml-lang.org/blog/

**Content Type**: Announcements, changelogs, feature posts
- New release announcements
- Language features explained
- Ecosystem updates
- Community highlights

**Value to Cortex**: Timeline of TML evolution, release notes context

**Indexing Strategy**:
- Consume via RSS feed: tml-lang.org/blog/rss.xml
- Index with publish date for temporal queries

### 6. Code Examples and Tutorials

**Source**: Embedded in docs, guides, and blog posts

**Content Type**: TML code samples
- Syntax examples
- Patterns and best practices
- Error handling demonstrations
- FFI examples

**Value to Cortex**: Training data for code generation, pattern matching

**Indexing Strategy**:
- Extract code blocks from MDX
- Tag with language/pattern (e.g., `code:tml:async`, `code:tml:ffi`)

## Recommended Cortex Integration Points

### Real-Time Sync
- **Database triggers**: When version_id changes in postgres, emit event
- **Webhook**: Registry API sends POST to Cortex on publish
- **Message queue**: Neon → Kafka/Redis → Cortex

### Batch Sync
- **Daily crawl**: Cortex crawls package.tml-lang.org/api/search API
- **Weekly sync**: Full rebuild of stdlib docs (on TML compiler release)

### Cache Strategy
- **TTL**: Package metadata cache (1 hour) — registry changes often
- **Invalidation**: stdlib docs re-cached on TML release tag
- **Search index**: Rebuild Cortex search index post-sync

## Data Ownership

| Data | Owner | Update Frequency |
|------|-------|------------------|
| Stdlib docs | TML compiler project | On release |
| Getting started guides | TmlDocs team | As-needed (docs PRs) |
| Language reference | TML language spec team | With language changes |
| Package metadata | Package authors (via publish) | Real-time |
| Blog posts | HiveLLM team | Weekly/monthly |
| Code examples | Community contributions | As-needed |

## Security Considerations

- **No private package data**: Registry is public-read (GitHub public repos only)
- **Audit results**: Include vulnerability severity (CVSS) for Cortex security analysis
- **Rate limiting**: Cortex crawler should respect registry rate limits (x-ratelimit headers)
- **API tokens**: No credentials exposed to Cortex (auth happens at registry edge)
