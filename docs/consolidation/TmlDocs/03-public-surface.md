# TmlDocs — Public Surface

## Deployment URLs

### Landing Page
- **Domain**: tml-lang.org
- **Hosting**: Vercel
- **Content**: Marketing, getting started, language documentation, blog

### Package Registry
- **Domain**: package.tml-lang.org
- **Hosting**: Vercel (frontend), Cloudflare Workers or Fly.io (backend API)
- **Content**: Package search, publish, audit, user profiles

## Output Formats

### Landing Page Output (tml-lang.org)

**Static/SPA files**:
- index.html, /docs/*, /blog/*, /guide/* (React SPA with client-side routing)
- CSS bundles (Tailwind compiled)
- JavaScript bundles (Vite-optimized with code splitting)
- Sitemap.xml, robots.txt
- Open Graph images (social preview)

**SEO Artifacts**:
- Meta tags (title, description, og:*, twitter:*)
- Structured data (JSON-LD for documentation schema)
- RSS feed for blog

### Package Registry Output

**API Responses** (JSON):
```json
{
  "name": "postgresql",
  "latest_version": "0.1.0",
  "repository": "https://github.com/hivellm/tml",
  "readme": "<rendered HTML>",
  "published_at": "2026-03-28T10:00:00Z",
  "versions": ["0.1.0"],
  "dependencies": []
}
```

**Frontend Artifacts**:
- Search results (interactive, paginated)
- Package detail pages (rendered README)
- User profile pages

## Performance Targets

| Metric | Target |
|--------|--------|
| Landing page LCP (Largest Contentful Paint) | < 1.5s |
| Package search latency | < 100ms |
| API response time (p95) | < 200ms |
| Lighthouse score (all categories) | > 95 |

## Accessibility Standards

- **WCAG 2.1 Level AA** compliance
- Dark/light mode support with system preference detection
- Keyboard navigation throughout
- Screen reader friendly (semantic HTML, ARIA labels)
