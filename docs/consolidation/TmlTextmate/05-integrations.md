# TmlTextmate — Integrations

## TML Language

TmlTextmate is the **official syntax grammar** for TML, a systems language with:

- **Type system**: algebraic data types (enum/type variants), generics with bounds
- **Paradigms**: functional (pattern matching), imperative (mutable state), concurrent (async/await)
- **Memory model**: ownership-based without lifetime annotations
- **Low-level access**: `lowlevel` blocks for FFI, SIMD, intrinsics
- **Async runtime**: async/await with Future support

Grammar tokenizes all TML constructs: types, behaviors (traits), pattern matching, decorators, memory annotations.

### TML Reference Features Covered

| Feature | Grammar Support |
|---------|-----------------|
| Type/enum definitions | YES (keywords + scoping) |
| Generic bounds | YES (pattern matching on [...]) |
| Pattern matching (when/else) | YES (keyword + patterns) |
| Decorators (@test, @extern) | YES (meta.decorator rules) |
| Template literals | YES (string interpolation) |
| Comments (///, //!) | YES (doc comment scopes) |
| Async/await | YES (keywords) |
| Lowlevel blocks | YES (keyword) |
| SIMD types (F32x4, I32x4) | YES (generic type parsing) |

## VSCode Ecosystem

### Discovery Mechanisms

1. **Language ID**: "tml" (inferred from fileTypes: ["tml"])
2. **File association**: VSCode recognizes .tml → source.tml → applies grammar
3. **Icon/badge**: theme integrations can add file icons
4. **Marketplace**: grammar appears in VS Code language support list (if contributed via extension)

### Theme Compatibility

Grammar follows standard TextMate scope conventions. Works with all standard themes:

- Dark+, Light+
- Dracula, One Dark Pro, Solarized
- Custom themes targeting scope names

### Editor Features (enabled by grammar)

- **Bracket matching**: `{...}`, `[...]`, `(...)`
- **Code folding**: comment blocks, function bodies, type definitions
- **Syntax error detection**: TextMate validator checks JSON schema
- **Outline/breadcrumbs**: VSCode infers from keyword/identifier scopes

## GitHub Linguist Integration

TmlTextmate is a **Linguist submodule** for:

1. **Language detection**: `.tml` files → marked as TML
2. **Repository statistics**: GitHub language breakdown includes TML
3. **Syntax highlighting on github.com**: TML code blocks highlighted
4. **Search indexing**: Linguist uses grammar for tokenization

### Linguist Configuration

Referenced in `languages.yml`:
```yaml
TML:
  type: programming
  color: "#3b63a3"
  extensions:
  - .tml
  ace_mode: tml
  tm_scope: source.tml
```

## Sample Code

6 reference `.tml` files demonstrate language features:

1. **hello_world.tml**: basic use/func/println
2. **types_and_behaviors.tml**: type/enum, impl, generics, behaviors, containers
3. **pattern_matching.tml**: when/else patterns, let-else, optional chaining (?.), ADTs
4. **generics_and_bounds.tml**: generic types with bounds, associated types (This::Output), Tree[T], where clauses
5. **async_http.tml**: async/await, Shared[Mutex[...]], concurrent task spawning, request handlers
6. **lowlevel_memory.tml**: @extern FFI bindings, lowlevel blocks, Arena allocator, SIMD, intrinsics, custom global allocators

Used for:
- **Testing grammar coverage** against real-world TML patterns
- **Documentation**: examples of TML syntax
- **VSCode preview**: when viewing `.tml` files, these patterns are highlighted
