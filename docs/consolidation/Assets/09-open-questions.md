# Open Questions & Gaps

## Missing Documentation

- **Brand guidelines**: color palette, typography, spacing rules
- **Asset roadmap**: planned additions (icons, illustrations, screenshots)
- **Usage policy**: when/where logos can be used (internal vs. public)
- **Attribution**: creator/designer info

## Technical Unknowns

- **Exact file sizes**: PNG compression level, dimensions in bytes
- **Color space**: RGB vs. RGBA, bit depth assumptions
- **PNG metadata**: embedded creation date, tool, copyright notices
- **Alpha channel**: exact transparency handling in dark variant

## Historical Context

- **Why these two only?** Were others considered?
- **When created?** No git history visible in this analysis
- **Who maintains?** No CODEOWNERS or maintainer documented

## Future Planning

- **Vector formats**: SVG versions planned?
- **Additional sizes**: need for 256×256, 128×128, favicon variants?
- **Other asset types**: where do icons, screenshots, diagrams live?
- **Multi-language assets**: localized versions needed?

## Recommendations for Future

1. Create `ASSETS.md` documenting purpose, naming convention, usage rights
2. Add `CHANGELOG.md` to track logo version history
3. Separate brand guidelines (could live here or in a dedicated Brand repo)
4. Automate size/format checks in CI
