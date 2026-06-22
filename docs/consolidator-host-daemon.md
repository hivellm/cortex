# Consolidator host daemon

The consolidator summarises through the **local, logged-in `claude` CLI**
(`claude -p`), not the Anthropic HTTP API (there is no API-key path). The
`claude` binary and its OAuth login live on the host, so the consolidator
**runs outside docker**. This doc covers running it as a host daemon that
resumes automatically after a machine reboot.

## Why outside docker

`claude.exe` and its login are host-local; they cannot be bridged into a
Linux container on a Windows host. The `cortex-consolidator` service in
`docker-compose.yml` is therefore behind the `manual` profile (excluded
from the default `docker compose up`) and exists only for image builds.

## Components

| Piece | Location | Role |
|-------|----------|------|
| Binary | `~/.cargo/bin/cortex-consolidator.exe` | `cargo install --path crates/cortex-workers --bin cortex-consolidator` |
| Wrapper | `scripts/consolidator-daemon.cmd` | sets env (data paths, host ports, `CLAUDE_CODE_BIN`) + runs `cortex-consolidator daemon` |
| Autostart | `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\cortex-consolidator-daemon.vbs` | launches the wrapper hidden at logon |
| Trigger source | classifier worker (`CORTEX_CONSOLIDATOR_TRIGGER_PRODUCER_ENABLED=true`) | emits `session_end` / `decision` / `topic` triggers into Synap `cortex.consolidator.triggers` |

The daemon pulls triggers from Synap (host port `17003`), summarises each
grain via the local `claude` CLI, and publishes the consolidation to
cortex-ingestion (host port `17010`).

## Host port map (container → host)

- Synap `15500` → `17003` (`SYNAP_BASE_URL=http://127.0.0.1:17003`)
- cortex-ingestion `17010` → `17010`
- Data: containers bind-mount `~/.cortex` at `/var/lib/cortex`, so the host
  daemon reads `~/.cortex/archive` + `~/.cortex/metadata.sqlite` directly.

## Autostart setup (no admin required)

`schtasks` (Task Scheduler) needs elevation in this environment, so the
daemon autostarts via the **Startup folder** instead — a `.vbs` launcher
that runs the wrapper hidden at user logon:

```vbs
CreateObject("WScript.Shell").Run "cmd /c ""E:\HiveLLM\Cortex\scripts\consolidator-daemon.cmd""", 0, False
```

Placed at
`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\cortex-consolidator-daemon.vbs`.
After a reboot, the daemon starts when the user logs in.

## Manual run / backfill

One-shot backfill of all eligible historical sessions (not just the last
24 h):

```sh
CORTEX_ARCHIVE_ROOT=~/.cortex/archive \
CORTEX_METADATA_DB=~/.cortex/metadata.sqlite \
CORTEX_INGESTION_URL=http://127.0.0.1:17010 \
CLAUDE_CODE_BIN="$(which claude)" \
cortex-consolidator nightly --dry-run false --all
```

`--dry-run true` previews the candidate set without invoking the CLI.

## Logs

- Daemon: `~/.cortex/consolidator-daemon.log`
- Backlog run: `~/.cortex/consolidator-run.log`
