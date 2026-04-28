# cortex-ops

> Spec: [`docs/specs/03-local-stack.md`](../../docs/specs/03-local-stack.md)

Operator CLI for the local Cortex stack. Two jobs: emit the bootstrap plan
that backends are seeded from, and probe each backend for liveness.

`cortex-ops` never mutates external state directly. Mutations are owned by
`cortex-api` (spec 04) and the workers (specs 05–08). This CLI is the
read-only seam that makes the desired state visible.

## Subcommands

### `plan`

Serializes the layout declared by `cortex-storage` (collections, Cypher,
indexes, streams) as JSON. The seed scripts under [`bin/`](../../bin/)
pipe this into each backend's native create API.

```bash
cortex-ops plan --pretty                  # all slices
cortex-ops plan --slice vectorizer        # collections only
cortex-ops plan --slice nexus             # bootstrap Cypher
cortex-ops plan --slice meilisearch       # index settings
cortex-ops plan --slice synap             # streams + KV namespaces
```

### `doctor`

Pings every backend at its configured URL and reports status. URLs default
to environment variables (`VECTORIZER_URL`, `NEXUS_URL`, `MEILI_URL`,
`SYNAP_URL`) and can be overridden with flags.

```bash
cortex-ops doctor
cortex-ops doctor --vectorizer-url http://localhost:17001 --nexus-url http://localhost:7474
```

Exit code is `0` only when every backend reports healthy.

## Install / build

```bash
cargo build --release -p cortex-ops
./target/release/cortex-ops plan --pretty
```

## Composition

```
cortex-storage  ──►  cortex-ops plan  ──►  bin/cortex-init.sh  ──►  backends
                                                                    (Vectorizer / Nexus / Meili / Synap)
```

`cortex-ops` does not bypass `cortex-storage`. Anything declared there is
emitted; anything not declared there is invisible to the bootstrap.

## Testing

```bash
cargo test -p cortex-ops
```

`doctor` smoke-tests run against the local docker-compose stack defined in
[`docker-compose.yml`](../../docker-compose.yml).
