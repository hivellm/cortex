# TmlTextmate — Public Surface

## VSCode Extension Integration

TmlTextmate is integrated into VS Code via TextMate grammar mechanism:

### Syntax Highlighting Capabilities

The grammar tokenizes and colorizes:

- **Keywords**: func, type, enum, behavior, impl, use, when, for, loop, async, await, lowlevel
- **Type annotations**: Generic syntax with brackets: `func foo[T: Ord + Display]()`
- **Pattern matching**: `when expr { Pattern => ... }`
- **Template literals**: `` `{interpolation}` `` with expr block support
- **Comments**: ///, //!, //, /* */
- **Decorators**: @test, @bench, @extern, @derive, @auto(duplicate), @intrinsic, @allocates
- **Numbers**: 0xFF, 0b1010, 0o755, 1.5, 1e-3, 42u32, 3.14f64
- **Operators**: ->, =>, |>, ?., ::, .., ?, !, and, or, not, xor, shl, shr
- **String escapes**: \n, \r, \t, \\, \", \x20, \u{1F600}

### Grammar Scope Names (TextMate convention)

Scope names follow the dotted naming convention:

| Scope | Tokens |
|-------|--------|
| `comment.line.documentation.tml` | ///, //! |
| `comment.line.double-slash.tml` | // |
| `comment.block.tml` | /* ... */ |
| `keyword.declaration.tml` | func, type, enum, impl, pub, use |
| `keyword.control.tml` | if, when, loop, async, await |
| `keyword.operator.word.tml` | and, or, not, as, is |
| `storage.type.primitive.tml` | I32, F64, Bool, Str, Char, Unit |
| `support.type.builtin.tml` | Vec, HashMap, Arc, Mutex, Maybe, Outcome |
| `string.quoted.double.tml` | "..." |
| `string.quoted.other.raw.tml` | r"..." or r#"..."# |
| `string.quoted.other.template.tml` | `` `...` `` |
| `constant.numeric.float.tml` | 1.5, 3.14f32 |
| `constant.numeric.integer.hexadecimal.tml` | 0xFF, 0xDEADBEEF |
| `meta.decorator.tml` | @test, @extern, @derive |

### VSCode Theming

Colors are resolved by VSCode's configured theme based on these scopes. Custom theme rules can target:

```json
{
  "editor.tokenColorCustomizations": {
    "comments": "#6A9955",
    "[keyword.control.tml]": { "foreground": "#D4D4D4" }
  }
}
```

## Marketplace Status

Not published as a standalone VSCode extension (grammar is embedded in Linguist and VSCode's built-in syntax support). Distribution via:

1. **GitHub Linguist**: Submodule for language detection
2. **TML compiler repo**: Bundled with official TML tools
3. **Manual**: Users can copy grammar to `~/.config/Code/User/syntaxes/tml.tmLanguage.json`
