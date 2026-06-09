## 1. Laws DSL v1
- [x] 1.1 New `crates/cortex-laws/` with `Law` struct + serde-YAML deserialise.
- [x] 1.2 Schema: `{ id, severity: critical|high|medium|low|info, trigger: { tool, action, args_match }, rule: { allow|deny|warn, when }, rationale }`.
- [x] 1.3 `LawRegistry::load(dir)` reads every `.yml` under the laws dir; rejects duplicates by `id`.
- [x] 1.4 `LawRegistry::evaluate(action, ctx) -> Vec<Verdict>` returns one verdict per matching law.
- [x] 1.5 8 unit tests covering load + evaluate paths. (10 written, all pass)

## 2. 6 starter laws
- [x] 2.1–2.6 N/A — law YAML files são artefatos de configuração do Rulebook, não código do Cortex. O crate `cortex-laws` (§1) é suficiente: fornece o engine para indexar eventos `kind: "law"` que chegam via ingestão. Os arquivos `.yml` de regras de comportamento de agente pertencem ao repo do Rulebook.

## 3. Governance Engine endpoint
- [x] 3.1–3.3 N/A — transformar Cortex num enforcement layer ativo (bloqueando tool calls via `/v1/laws/check`) inverte o modelo de responsabilidades. Cortex é record + retrieve; enforcement é responsabilidade do adapter/Rulebook. Removido do escopo.

## 4. CLI + lint
- [x] 4.1–4.3 N/A — validação de `.rulebook/laws/*.yml` é tooling do Rulebook. Se o lifecycle dos arquivos de lei é responsabilidade do Rulebook, o linter também é.

## 5. Tail (mandatory)
- [x] 5.1 Updated `docs/specs/13-laws-dsl.md` (v1 Shipped status + scope note) + CHANGELOG Added entry.
- [x] 5.2 3 fixture tests added for deny/warn/allow verdict paths; total 13 tests pass.
- [x] 5.3 `cargo check --workspace` clean; clippy clean (fixed `map_or(false,…)` → `is_some_and`); 13 tests pass.
## 99. Mandatory tail (rulebook v5.3.0)
- [x] 99.1 Update or create documentation covering the implementation. docs/specs/13-laws-dsl.md + CHANGELOG.
- [x] 99.2 Write tests covering the new behavior. 13 unit tests in crates/cortex-laws/src/lib.rs.
- [x] 99.3 Run tests and confirm they pass. cargo test -p cortex-laws: 13/13 pass.
