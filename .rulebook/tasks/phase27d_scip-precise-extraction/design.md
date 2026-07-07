# phase27d — real rust-analyzer SCIP schema (captured 2026-07-07)

The §1.1 gating knowledge, captured from REAL output — not a
from-memory fixture, per the task's own warning. Method: a minimal
2-module fixture crate (`Worker` in `lib.rs` calling
`storage::Store`), indexed with `rust-analyzer scip .`
(rust-analyzer 1.96.0, installed via `rustup component add
rust-analyzer`), decoded with `protoc --decode_raw`.

## Output format reality

`rust-analyzer scip` emits **binary protobuf** (`index.scip`), NOT
JSON. The tasks.md §2.1 wording "parse a SCIP index (JSON)" does not
match reality; the options for the parser are:

1. Parse the protobuf directly (prost + the public
   `sourcegraph/scip` `scip.proto`) — no external tool dependency.
2. Require the Sourcegraph `scip` CLI (`scip print --json`) as a
   conversion step — extra tool, but human-inspectable intermediates.

Option 1 is the right call for the bootstrap/CI path (§2.4): one
fewer binary to install, and the schema below is small.

## Wire schema (field numbers from decode_raw, names from scip.proto)

```
Index (top level)
├─ 1: Metadata
│    ├─ 2: ToolInfo { 1: name = "rust-analyzer", 2: version }
│    ├─ 3: project_root  ("file://C:\\..." — file URI, BACKSLASHES on Windows)
│    └─ 4: text_document_encoding = 1 (UTF-8)
├─ 2: repeated Document
│    ├─ 1: relative_path  ("src\\lib.rs" — BACKSLASH separators on Windows!)
│    ├─ 4: language = "rust"
│    ├─ 2: repeated Occurrence
│    │    ├─ 1: range      — packed varints; 3 elems = same-line
│    │    │                  [line, startChar, endChar]; 4 elems =
│    │    │                  [startLine, startChar, endLine, endChar]
│    │    ├─ 2: symbol     — the symbol string (grammar below)
│    │    ├─ 3: symbol_roles — bitfield; 1 = Definition; ABSENT (0)
│    │    │                  for plain references
│    │    └─ 7: enclosing_range — packed varints, same encoding
│    ├─ 3: repeated SymbolInformation
│    │    ├─ 1: symbol
│    │    ├─ 3: documentation (doc-comment text; repeated)
│    │    ├─ 5: kind (numeric enum — observed: Function=17, run/26?,
│    │    │         Crate=29, Parameter=37, SelfParam=44, Struct=49,
│    │    │         impl=55, Method=80, Field=15 — verify against
│    │    │         scip.proto's SymbolInformation.Kind before use)
│    │    ├─ 6: display_name
│    │    ├─ 7: signature_documentation { 4: language, 5: text, 6: ? }
│    │    └─ 8: enclosing_symbol (locals only — e.g. "local 0" →
│    │         its containing method symbol)
│    └─ 6: position_encoding = 1
└─ (3: repeated external SymbolInformation — not emitted for this
      fixture; cross-crate refs appear inline in occurrences instead)
```

## Symbol grammar (observed)

```
rust-analyzer cargo <package> <version> <descriptors>
  e.g. "rust-analyzer cargo scip-fixture 0.1.0 storage/Store#"
       "rust-analyzer cargo scip-fixture 0.1.0 impl#[Worker]new()."
       "rust-analyzer cargo scip-fixture 0.1.0 Worker#store."
local <N>
  e.g. "local 0"  (function-scoped; pair with SymbolInformation
                   field 8 enclosing_symbol to resolve context)
```

Descriptor suffix conventions (SCIP standard):
- `name/`  namespace/module (`crate/`, `storage/`)
- `Name#`  type (struct/enum/trait)
- `name().` method/function
- `name.`  term (field, const)
- `impl#[Type]…` impl-block scoping — note methods resolve under
  `impl#[Worker]new().`, NOT `Worker#new().` — the resolver's
  same-document precedence must treat `impl#[T]m().` and `T#` as the
  same logical owner when emitting `DEFINES` edges.

**Cross-crate references** carry the full dependency descriptor
inline in the occurrence, INCLUDING a URL package segment:

```
rust-analyzer cargo core https://github.com/rust-lang/rust/library/core ops/arith/impl#[usize][`Add<Self>`]add().
```

i.e. `<scheme> <manager> <pkg-name> <pkg-url-or-version> <descriptors>`
with the package field itself space-splitting into name + url for
stdlib crates. The §2.2 two-pass resolver must therefore split the
symbol string carefully: the FIRST two tokens are scheme+manager, the
descriptors are the LAST space-separated segment, and everything
between is the package identity (1..2 tokens). Symbols whose package
identity is not the indexed crate map to `scip_external` stub nodes
(§2.3) rather than dangling edges.

## Gotchas for the implementation

1. **Windows path separators**: `relative_path` uses `\\` on Windows.
   Normalise to `/` before joining against repo-rooted artifact paths
   (the graph's `Artifact` natural keys use forward slashes).
2. **Range encoding is positional**: 3-element = single-line
   shorthand. Do not assume 4 elements.
3. **symbol_roles is a BITFIELD** (Definition=1, Import=2,
   WriteAccess=4, ReadAccess=8, Generated=16, Test=32) — check via
   `roles & 1 != 0`, never `== 1`.
4. **Locals** ("local N") are document-scoped — never emit graph
   nodes for them; they'd collide across documents.
5. **rust-analyzer runs cargo metadata** on first index (network
   fetch possible) — CI wiring (§2.4) needs a warmed cargo cache or
   offline flag.

## Fixture reproduction

```sh
# fixture crate: two modules, cross-module call, doc comments
rust-analyzer scip <fixture-dir>   # writes index.scip (protobuf)
protoc --decode_raw < index.scip   # field-number view (this doc)
```

A committed JSON-ified fixture for parser unit tests should be
generated by the parser itself once written (round-trip: parse the
real index.scip → serialise to the internal model → snapshot).
