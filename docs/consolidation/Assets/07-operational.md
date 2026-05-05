# Operational & Distribution

## Versioning Strategy

**Git-based versioning**:
- Versions tied to Git commits (no semantic versioning tags observed)
- Changes tracked in Git history (standard GitHub)
- No release notes or CHANGELOG

## Distribution Channel

**GitHub raw content URL**:
```
https://raw.githubusercontent.com/hivellm/Assets/main/<filename>
```
- **Branch**: `main` (implicit via URLs)
- **CDN**: GitHub's edge network (no custom CDN layer)
- **Caching**: subject to GitHub's HTTP caching headers

## Update Process

1. Developer commits new/modified asset
2. Git push to `main`
3. Immediately available via raw content URL (no build step)
4. Consumers must update hardcoded URLs if structure changes

## No Formal Release

- No GitHub Releases / Tags
- No version numbering
- No deprecation policy

## Operational Load

**Minimal**: static files, no compute, no maintenance cadence.
