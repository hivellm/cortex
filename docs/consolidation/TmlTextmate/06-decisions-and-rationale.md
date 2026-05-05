# TmlTextmate — Decisions & Rationale

## Grammar Format: TextMate vs. Alternatives

**Decision**: Use TextMate grammar (JSON format) instead of YAML, PEG, or language-specific grammars.

**Rationale**:
1. **Standard format**: TextMate grammars are industry standard (VS Code, Sublime, Atom, Linguist)
2. **Maintenance**: single source-of-truth for all editors
3. **GitHub Linguist compatibility**: required for repository language detection
4. **Regex-based**: pattern matching is sufficient for syntax coloring (not full parsing)
5. **Ecosystem**: extensive tool support (validators, converters, documentation)

**Trade-off**: regex-based patterns cannot handle arbitrarily nested structures; mitigated by recursion rules and `include: $self`.

## Pattern Precedence Order

**Decision**: Comments and decorators highest; operators/punctuation lowest.

**Rationale**:
1. **Comments must not highlight keywords inside** — doc comments (//) override keyword rules
2. **Decorators (@) are rare, specific** — match before generic identifier rules
3. **Keywords before types** — "let" is keyword, not identifier
4. **String literals before number literals** — avoid partial matches (e.g., "0x" in string)
5. **Operators last** — catch edges not handled by specialized rules

## Template Literal Syntax Design

**Decision**: Backtick templates `` `{expr}` `` with nested pattern recursion.

**Rationale**:
1. **Mirrors Rust/JavaScript**: familiar to systems programmers
2. **Distinguishes from generic brackets**: `<...>` (if any) vs. `` `...` ``
3. **Nested expressions via `include: $self`**: allows arbitrary TML expressions inside `{...}`
4. **Escape sequences**: standard Unicode escape support (\u{1F600}, \x20)

**Alternative considered**: $-based interpolation (Python-style). Rejected — backticks are more visually distinct and familiar.

## Decorator Parsing

**Decision**: Permissive decorator syntax with optional arguments.

**Rationale**:
1. **@name alone**: @test, @bench, @extern
2. **@name(args)**: @extern("c"), @auto(duplicate), @derive(Clone, Copy)
3. **Arguments can be strings or identifiers**: covered by separate rules
4. **No strict validation**: grammar allows future decorators without updates

## Type System Representation

**Decision**: Explicit primitive types + separate builtin types list.

**Rationale**:
1. **Primitives (I32, F64)**: low-level, always available, colored distinctly
2. **Builtins (Vec, HashMap)**: generic containers, standard library imports, different visual precedence
3. **Custom types**: handled by identifier rules, no hardcoding
4. **Generic syntax `[T: Bound]`**: pattern match on brackets, not keyword-dependent

## Raw String Syntax

**Decision**: Support r#"..."# with variable-length hash delimiters.

**Rationale**:
1. **Matches Rust convention**: familiar to systems programmers
2. **Recursive rule**: `r(#*)"\` captures 0 or more hashes
3. **End delimiter**: `"(\1)` matches same number of closing hashes via backreference
4. **Escape-free strings**: raw strings bypass escape processing (useful for regexes, paths)

## Scope Naming Convention

**Decision**: Follow TextMate standard dotted naming: `category.subcategory.language`.

**Rationale**:
1. **Hierarchical resolution**: themes can target any level (e.g., all `keyword.*` or specific `keyword.control.tml`)
2. **Language-specific suffixes** (.tml): avoid conflicts with other grammars in multi-language documents
3. **Semantic grouping**: `meta.decorator.tml`, `storage.type.primitive.tml` communicate intent
4. **Extensibility**: new rules follow pattern without breaking existing theming

## No Code Generation

**Decision**: Grammar is hand-written, not code-generated.

**Rationale**:
1. **Transparency**: maintainers understand every pattern
2. **Debugging**: regex issues are visible and testable
3. **Small size**: ~8 KB is manageable
4. **Revision control**: diffs are human-readable
5. **TML language stability**: grammar updates only when language evolves

## Integration with Linguist

**Decision**: TmlTextmate is a submodule in GitHub Linguist.

**Rationale**:
1. **Single source of truth**: changes in TmlTextmate automatically propagate to GitHub language detection
2. **Community maintenance**: Linguist handles registry, caching, version updates
3. **No duplication**: avoids maintaining separate language definitions in multiple repos
4. **Public exposure**: TML repos automatically get syntax highlighting on github.com
