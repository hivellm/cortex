# TmlTextmate — Architecture

## Grammar Structure

The grammar (`syntaxes/tml.tmLanguage.json`) uses a repository-based TextMate pattern system:

### Root Level
- **`$schema`**: JSON schema URL (martinring/tmlanguage)
- **`name`**: "TML"
- **`scopeName`**: "source.tml" (primary scope identifier)
- **`fileTypes`**: ["tml"]
- **`patterns`**: Array of rule includes (ordering determines precedence)

### Pattern Organization (by precedence)

1. **Comments** (highest): block comments, doc comments, line comments
2. **Decorators**: @test, @bench, @extern, @derive, etc.
3. **Preprocessor**: C-style directives (#if, #ifdef, #define)
4. **String literals**: template backtick literals, raw strings, double-quoted strings
5. **Numbers**: float, hex, binary, octal, decimal (with type suffixes)
6. **Keywords**: declaration, control flow, operators, other
7. **Types**: primitive (I8–I128, U8–U128, F32, F64, Bool, Str, Char, Unit, Never)
8. **Built-in types**: generic containers (Maybe, Vec, HashMap, Arc, Mutex, etc.)
9. **Constants**: boolean, null, Maybe/Outcome variants
10. **Functions/Types**: definition patterns
11. **Operators & punctuation** (lowest): to avoid masking higher-priority rules

### Repository Rules

**Key patterns:**

- **Template strings** (`` `...{expr}...` ``): nested pattern matching with `\{...\}` expression blocks
- **Raw strings** (r#"..."#): variable-length hash delimiters
- **String interpolation** ($var, ${expr}): for double-quoted strings
- **Decorators** (@name(...)): attribute parsing
- **Numbers**: all bases with optional type suffixes (u8, i32, f64, etc.)
- **Keywords**: matches word boundaries to avoid partial matches

## Scope Names

Scope chain pattern: `<category>.<type>.<language>`

Examples:
- `comment.block.tml` — block comments
- `keyword.declaration.tml` — func, type, enum, impl, etc.
- `storage.type.primitive.tml` — I32, F64, Bool, Str
- `string.quoted.double.tml` — double-quoted strings
- `constant.numeric.integer.hexadecimal.tml` — 0xFF, 0x1A
- `meta.decorator.tml` — @test, @derive attributes
- `support.type.builtin.tml` — Vec, HashMap, Arc, Mutex
