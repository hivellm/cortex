# Integrations & Consumers

## Known Consumers

### Direct References

**Cortex** (this project):
- GUI may reference logo for branding/splash
- Documentation may embed or link logo

**Other HiveLLM Projects** (potential):
- Rulebook, Vectorizer, Nexus, Synap, Lexum, Expert
- Any that include README.md badges, splash screens, or branding

### Dependency Pattern

- **Pull model**: projects fetch via raw GitHub URLs (no SDK)
- **No push notification**: no webhook or cache invalidation
- **Static reference**: typically hardcoded URLs in markdown or HTML

## Integration Depth

**Very shallow**: read-only asset consumption. No bidirectional communication, no API dependencies, no service integration.

## CI/CD Implications

- Build systems may fetch logos during documentation generation
- No build-time validation of asset availability observed
