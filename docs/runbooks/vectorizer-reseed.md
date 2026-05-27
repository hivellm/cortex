# Vectorizer re-seed runbook

## Root cause (2026-05-27)

Vectorizer 3.x writes its persistent state to the XDG data dir
(`/.local/share/vectorizer/`), **not** to `/data`. The `/data` mount
in `docker-compose.yml` only holds `config.yml` plus a placeholder
`data/vectorizer.vecdb` (empty). Until 2026-05-27 the XDG path lived
in the container's writable layer, so every `docker compose up -d
vectorizer` (re-create, not restart) wiped every collection.

Symptoms:

- `/v1/status` reports `coverage: { vectorizer: missing: 565/567 }`.
- `cortex_vector_search` against any collection except a freshly
  re-ingested one returns 404 `collection_not_found`.
- `cortex_similar_sessions` returns empty for every query.
- The `cortex-cortex-consolidations` collection is gone even though
  the source data still exists in Meili + the archive.

## Structural fix

`docker-compose.yml` now mounts a second persistent volume on the
XDG path:

```yaml
vectorizer:
  volumes:
    - vec-data:/data
    - vec-state:/.local/share/vectorizer    # added 2026-05-27
```

Named volume `vec-state` is declared at the bottom of the file
alongside `vec-data`. From now on, `docker compose up -d vectorizer`
preserves every collection across recreates.

Verify the mount after a recreate:

```sh
docker inspect cortex-vectorizer \
  --format '{{range .Mounts}}{{.Destination}}={{.Source}}{{println}}{{end}}'
```

Expected output includes both `/data=…/vec-data/_data` and
`/.local/share/vectorizer=…/vec-state/_data`.

## Re-seed after the wipe

After mounting the volume, the existing collections are still empty.
Re-seed flows through the classifier → embedder → fulltext pipeline:

1. Ensure cortex-ingestion is healthy and the archive root is mounted
   on the container:
   ```sh
   docker compose ps cortex-ingestion meilisearch vectorizer
   ```
2. Restart the embedder worker so it re-reads the archive's
   un-embedded partitions. The worker is supposed to back-fill any
   collection it cannot find:
   ```sh
   docker compose restart cortex-embedder-worker
   docker logs -f cortex-embedder-worker | grep -E 'collection|insert'
   ```
3. For repos with no live envelopes (i.e. nothing in the archive),
   run `cortex-bootstrap` from the host:
   ```sh
   cortex-bootstrap --repo /path/to/Repo --graph-static
   ```
   This walks the repo's source tree, classifies + embeds every
   artifact, and posts to ingestion → classifier → embedder, which
   writes into Vectorizer.
4. Confirm coverage recovery:
   ```sh
   curl -sS http://127.0.0.1:17000/v1/status \
     | python3 -c 'import json,sys; d=json.load(sys.stdin); \
       print(d["daemon"]["coverage"]["backends"])'
   ```

## Order of operations on a wipe-out

If `/v1/status` already shows `missing: <pct>` ≥ 5% after a deploy:

| step | command                                                 |
|------|---------------------------------------------------------|
| 1    | `docker compose up -d vectorizer` (mount already wired) |
| 2    | `docker compose restart cortex-embedder-worker`         |
| 3    | wait 10–30 min depending on archive size                |
| 4    | re-run `scripts/phase20_acceptance.sh` and confirm §2/3 |

If §2/3 still FAIL after step 3, fall back to per-repo `cortex-bootstrap`.

## Related files

- `docker-compose.yml` — the mount declaration.
- `crates/cortex-workers/src/embedder/worker.rs` — re-ingest entry
  point that reads `cortex.events.enriched` and writes to Vectorizer.
- `crates/cortex-cli/src/bootstrap/publisher.rs` —
  `cortex-bootstrap` synthesizes envelopes onto `cortex.events.bootstrap`
  which the classifier worker then forwards to enriched.
