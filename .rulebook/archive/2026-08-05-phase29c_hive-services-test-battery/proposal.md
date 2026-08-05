# Proposal: phase29c_hive-services-test-battery

Source: user request 2026-08-03 ("acabei de atualizar o vectorizer
tambem para uso do thunder, atualiza tanto o server no docker e o sdk,
assim vamos iniciar uma bateria de testes no synap, nexus e vectorizer
pra ve se esta tudo ok ou se tem algum erro, faz uma task pra isso,
caso encontre algum bug abra uma issue no repositorio do respectivo
projeto para correcao").

## Why

Three Hive services moved underneath Cortex within one week
(Vectorizer 3.5 -> 3.6 "Thunder" rpc stack, Synap 1.0 -> 1.3, Nexus
2.5). Each previous bump surfaced real regressions only when Cortex
exercised the service (nexus#29 `_id` projection, undirected-pattern
zero-rows, synap ephemeral-room ERROR spam). This task runs a
deliberate test battery against all three LIVE services so the bugs
are found by us now, not by retrieval quality later — and every
confirmed bug gets an upstream issue in the owning repo.

## What Changes

- Vectorizer SDK 3.5.0 -> 3.6.0 (workspace pin + lockfile) and
  container 3.5.0-fastembed -> 3.6.0-fastembed; cortex images rebuilt
  on the new SDK; data-volume integrity verified after recreate.
- Test battery (tasks section 2): per-service probe suites executed
  against the live stack, covering the operations Cortex actually
  depends on + the previously-found regression classes.
- Issue-filing protocol: every CONFIRMED bug (reproducible, not a
  Cortex-side misuse) gets a `gh issue create` on hivellm/vectorizer,
  hivellm/synap, or hivellm/nexus with the reproduction inline.

## Impact

- Affected specs: docs/specs/03-local-stack.md (pin), CHANGELOG.
- Affected code: Cargo.toml/lock, docker-compose.yml; no behavior
  changes expected in Cortex (this is a verification round).
- Breaking change: NO.
- User benefit: upstream bugs caught at bump time with reproductions,
  tracked in the owning repos.
