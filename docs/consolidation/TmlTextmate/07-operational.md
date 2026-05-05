# TmlTextmate — Operational

## Build & Distribution

### Grammar Validation

**Tool**: TextMate Language JSON schema validator

**Command** (manual):
```bash
curl -s https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json | \
  jq . > /tmp/schema.json
jq . syntaxes/tml.tmLanguage.json > /dev/null && echo "Valid JSON"
```

**Validation points**:
- JSON syntax correctness
- Required fields: name, scopeName, patterns, repository
- Pattern objects: begin/end paired, captures indexed, include references resolved
- No circular includes in repository

### Local Development

**Installation for VSCode testing**:

1. Clone TmlTextmate repo
2. Copy/symlink `syntaxes/tml.tmLanguage.json` to:
   - **Linux/macOS**: `~/.config/Code/User/syntaxes/tml.tmLanguage.json`
   - **Windows**: `%APPDATA%\Code\User\syntaxes\tml.tmLanguage.json`
3. Restart VSCode or reload (`Ctrl+R` in window)
4. Open a `.tml` file; grammar should activate

**Testing**:
- Edit grammar JSON
- Open `samples/*.tml` files
- Observe syntax highlighting changes
- Check scopes with VSCode "Inspect TM Scopes" (Cmd+Shift+P)

### Release Process

**Versioning**: Semantic versioning (follow TML language version)

**Steps**:
1. Update `tml.tmLanguage.json` with new patterns
2. Test against sample files
3. Commit with message: `feat(grammar): add <feature>`
4. Create git tag: `v<version>` (e.g., v0.1.0)
5. Push to GitHub (automatic Linguist submodule update follows)

**Linguist synchronization**:
- Linguist periodically pulls TmlTextmate updates
- No manual registry update needed (git submodule mechanism)
- Changes appear on github.com within 24-48 hours

### Sample File Maintenance

**Samples directory**: `samples/`

**Purpose**: 
- Demonstrate language features
- Validate grammar coverage
- Serve as documentation

**Update process**:
1. When TML language gains feature, add sample code
2. Ensure grammar highlights correctly
3. Update sample file comments
4. Commit alongside grammar changes

**Sample files** (6 total):
- `hello_world.tml` — ~7 lines, basic syntax
- `types_and_behaviors.tml` — ~140 lines, ADTs, generics, traits
- `pattern_matching.tml` — ~120 lines, when/else, let-else, guards
- `generics_and_bounds.tml` — ~150 lines, associated types, where clauses, trees
- `async_http.tml` — ~110 lines, async/await, locks, concurrency
- `lowlevel_memory.tml` — ~140 lines, FFI, SIMD, intrinsics, allocators

## Troubleshooting

### Grammar not activating in VSCode

**Cause**: Grammar not installed or file not recognized

**Fix**:
1. Verify file extension is `.tml`
2. Check `File > Preferences > Settings > Text Editor: Language Mode` — should show "TML"
3. Inspect scopes: `Cmd+Shift+P > Inspect TM Scopes` — should show `source.tml`

### Incorrect tokenization

**Cause**: Pattern order or regex issue

**Debug steps**:
1. Isolate offending line in sample file
2. Compare against similar constructs in other samples
3. Check TextMate scope inspector for actual scope assigned
4. Verify pattern regex with online regex tester (JavaScript flavor)
5. Check for ambiguous pattern overlaps

### Linguist not detecting .tml files

**Cause**: Linguist cache not refreshed

**Fix**:
1. Verify TmlTextmate submodule is latest version
2. Force Linguist update: ping GitHub with `.gitattributes` change (adds dummy line, reverts)
3. Wait 24-48 hours for public GitHub.com cache invalidation

## Maintenance Burden

**Low**:
- Grammar is stable; TML language changes are infrequent
- No runtime dependencies (pure JSON)
- No CI pipeline required (manual validation sufficient)
- Sample files are documentation, not code under test

**Update triggers**:
- TML compiler adds new keyword or syntax
- Bug reported in highlighting
- New operator or literal form introduced in TML

**Typical update**: 5-10 minutes (edit JSON, test, commit)
