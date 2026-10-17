# Self-contained daily plugin installation

## Ownership and priority

On 2026-10-17 the operator explicitly selected: "Assumir phase35; Claude revisa".
This task takes priority over the old backlog for this one execution. Codex
coordinates implementation and commits; Claude reviews. Execute every item in
the existing checklist in order. The earlier alternative phase34 plan remains
superseded and is not an implementation input.

The initial shared-checkout baseline is commit `27df2a7`; the existing change
to `.rulebook/tasks/README.md` came from task scaffolding and is preserved.
Installed daily API/adapter and private client settings remain the operational
baseline until the checklist reaches verified migration and acceptance.

The native Claude `SendMessage` tool delivered the ownership notice to peer
`cortex-73`, receipt `68e807c0-d8ae-4773-97fd-8f4c351b3a1f`, at
2026-10-17T15:07:24Z. The receipt confirms inbox delivery, not that the peer has
read or accepted the notice. Raw evidence stays in ignored
`.cortex-state/phase35/coordination/`.

## Configuration selection

Runtime configuration selects one file: nonempty `CORTEX_CONFIG_FILE`, otherwise
an existing `<Cortex home>/config.toml`, otherwise `./cortex.toml`. Environment
values override that file; explicit CLI overrides remain highest. Files are
alternatives, not merged layers: selecting user configuration must not import
unexpected runtime settings or credentials from the current repository.

`CORTEX_HOME` selects the Cortex home directory. Without it, use the platform
user home plus `.cortex`. Empty selector variables behave as unset. Existing
explicit-file loader APIs remain deterministic and do not consult ambient user
configuration. Missing explicit files retain the existing defaults-plus-env
behavior; malformed or unreadable selected files report their error.

Repository bootstrap and repository identity continue to read their own
`cortex.toml` through the separate `CortexToml` and scope parsers. Global runtime
configuration does not replace the repository identity/configuration contract.

## Verification boundary

Configuration tests use injected environment snapshots or subprocess isolation,
never mutate process-wide environment while parallel tests run. No installed
user configuration is written while developing discovery. Later credential,
setup, lifecycle and migration work must preserve the observed phase34 memory
delivery/authentication contracts and the release-build operational requirement.
