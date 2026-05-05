# TmlTextmate — Cortex Relevance

## Cortex Ingestion Priority

**Priority**: LOW (read-heavy, documentation artifact)

**Reason**: TmlTextmate is a **reference grammar**, not operational code. Cortex should index it for:
1. **Language feature discovery** — extract keyword/type lists
2. **Syntax documentation** — operator/literal reference
3. **Sample code corpus** — training/validation on TML syntax patterns

## Data to Index

### High-Value Extraction

1. **Keyword corpus**: declaration, control, operators, other (49 keywords total)
   - Source: `repository.keywords_*` in grammar
   - Use: TML language feature completeness check

2. **Type system**: primitives (12) + builtins (42)
   - Source: `repository.types_primitive`, `repository.types_builtin`
   - Use: type signature validation in Cortex analysis

3. **Scope taxonomy**: TextMate scope names
   - Source: all `name` fields in patterns
   - Use: syntax highlighting consistency check across HiveLLM editors

4. **Sample code**: 6 `.tml` files
   - Source: `samples/*.tml`
   - Use: train Cortex tokenizers/classifiers on TML syntax

### Medium-Value Extraction

5. **Pattern rules**: regex patterns for each language construct
   - Source: all `match`, `begin`, `end` fields
   - Use: validate Cortex's own TML parsing logic

6. **Decorators**: attribute list (@test, @extern, @derive, etc.)
   - Source: `meta.decorator.tml` pattern
   - Use: TML annotation completeness

### Low-Value (Skip)

- JSON structure validation rules (TextMate schema details)
- VSCode integration specifics (irrelevant to Cortex backend)

## Ingestion Method

### Option 1: Direct Grammar Indexing (Recommended)

```
Index resource: TmlTextmate::Grammar
Type: syntax-definition
Schema: 
  keywords: [String]
  primitives: [String]
  builtins: [String]
  scopes: {name -> [tokens]}
  samples: [file_path]
Source: syntaxes/tml.tmLanguage.json
Embedding: keyword + type embeddings for similarity search
```

**Search queries Cortex can answer**:
- "What decorators does TML support?" → @test, @bench, @extern, ...
- "Is `Maybe` a TML builtin type?" → Yes (support.type.builtin.tml)
- "Find TML code samples using async/await" → async_http.tml
- "What are TML primitives?" → I8, I16, ..., Bool, Str

### Option 2: Sample Code Corpus

```
Index resource: TmlTextmate::Samples
Type: code-corpus
Schema:
  filename: String
  topic: String (types, patterns, generics, async, lowlevel)
  length: Int
  keywords_used: [String]
  types_used: [String]
Content: full .tml text
```

**Use cases**:
- Train Cortex tokenizers on TML syntax
- Validate Cortex's code analysis against real patterns
- Generate synthetic TML code for testing

### Option 3: Scope Taxonomy Reference

```
Index resource: TmlTextmate::ScopeTaxonomy
Type: reference
Schema:
  scope_name: String (e.g., "keyword.control.tml")
  tokens: [String]
  category: String
  theme_variable: String (for VSCode/Sublime integration)
Content: TextMate scope hierarchy
```

**Use**: Ensure Cortex's semantic analysis uses consistent scopes across HiveLLM tools.

## Ingestion Workflow

1. **Read** `syntaxes/tml.tmLanguage.json` via Cortex FileIngester
2. **Extract** keyword, type, and scope lists (JSON path queries)
3. **Parse** sample .tml files (tokenize, AST)
4. **Embed** extracted data (BM25 + dense embeddings)
5. **Store** in Cortex graph as:
   - Node: `Language::TML` with properties: keywords, types, decorators
   - Edge: `Language::TML` → `SyntaxGrammar::TmlTextmate`
   - Node: `SyntaxGrammar::TmlTextmate` with properties: scopes, samples
   - Nodes: `Sample::hello_world`, `Sample::async_http`, etc.

## Cortex Query Examples

Once indexed:

```cortex
MATCH (l:Language {name: "TML"}) 
  -[:HAS_GRAMMAR]-> (g:SyntaxGrammar {name: "TmlTextmate"})
RETURN g.keywords, g.builtin_types
// Answer: [func, type, impl, ...], [Vec, HashMap, Arc, ...]

MATCH (s:Sample {language: "TML"})
WHERE s.topics CONTAINS "async"
RETURN s.filename, s.content
// Answer: ["async_http.tml", ...]

MATCH (t:Type {language: "TML"})
WHERE t.category = "builtin"
RETURN t.name
// Answer: [Vec, HashMap, Maybe, Outcome, ...]
```

## Estimated Ingestion Cost

- **Grammar file size**: ~8 KB
- **Sample files**: ~600 KB total
- **Parsing time**: <1 second
- **Storage (indexed)**: ~50 KB (compressed scopes + keywords + embeddings)
- **Storage (full)**: ~600 KB (with samples)

## Recommendation

**Ingest as read-heavy documentation artifact**:
1. Index grammar for keyword/type lookups
2. Index samples for code pattern training
3. Use for TML syntax validation in Cortex analysis
4. Do NOT require real-time grammar validation (grammar is stable, TML changes infrequently)
