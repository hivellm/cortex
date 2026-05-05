# Decisions & Rationale

## Branding Decisions

### Why Two Variants?

- **Light variant** (`512x512.png`): default for standard backgrounds
- **Dark variant** (`512x512-dark.png`): for dark/inverted theme contexts
- **Rationale**: maintain visual consistency across light and dark UIs

### Why PNG?

- **Lossless compression**: preserves logo quality (important for branding)
- **Transparency support**: alpha channel for flexible placement
- **Web-native**: broadly supported, no special rendering required
- **File size**: reasonable for web distribution

### Why 512×512?

- **Common size**: fits multiple use cases (social media, badges, documentation)
- **Scalable down**: can be reduced for smaller contexts without loss
- **Retina-ready**: adequate DPI for high-resolution displays

## No SVG Support

**Decision**: vector (SVG) logos not currently included.
- **Rationale unknown**: likely design tools/sources are proprietary or elsewhere
- **Trade-off**: PNG is fixed-resolution; larger sizes may pixelate

## Repository Structure

**Minimal/flat**: no subdirectories.
- **Rationale**: only 2 files; no complexity warrants hierarchy
- **Scalability risk**: future assets (icons, banners) may require restructuring
