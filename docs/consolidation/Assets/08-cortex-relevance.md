# Cortex Relevance & Indexing

## Should Cortex Index Assets?

**Priority: LOW**

### Why Low Priority

1. **Static content**: logos do not change frequently or require indexing
2. **Read-only access**: no discovery or search needed (URLs are hardcoded)
3. **No metadata**: files lack descriptions, tags, or queryable content
4. **Small scope**: only 2 assets; not worth crawler overhead

### Potential Value

- **Brand monitoring**: track logo updates across projects
- **Asset inventory**: catalog all HiveLLM branded materials
- **Link resolution**: map which projects reference which logos

### Recommended Approach

**Skip indexing Assets repository itself**, but:

- **Capture references** when Cortex encounters logo URLs in other projects' documentation
- **Metadata entry**: record known Asset URLs in Cortex graph (without deep indexing)
- **Light touch**: link to this consolidation KB rather than full crawl

### If Indexing Were Needed

Would need:
- Metadata file (JSON/YAML) describing each asset
- Version history or release notes
- Usage guidelines or context

None currently exist.
