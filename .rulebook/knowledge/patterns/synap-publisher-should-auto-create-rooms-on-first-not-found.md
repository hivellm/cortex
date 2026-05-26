# Synap publisher should auto-create rooms on first "not found"

**Category**: architecture
**Tags**: synap, publisher, self-healing, stream.create, cortex-bootstrap, cortex-classifier-worker

## Description

Synap 0.11 requires `stream.create` before any `stream.publish` call against a previously-untouched room. Publishers that hard-fail on the first publish create operational ordering constraints (the producer cannot start before someone has already touched the room). The clean fix is to make every Synap publisher self-heal: on a "Room not found" / "Invalid request: Room ..." error, call `streams.create_room(room, None)` once and retry the publish. This is what cortex-bootstrap and cortex-classifier-worker now do; cortex-ingestion/embedder/graph/fulltext should follow.

## Example

match self.handle.streams().publish(room, kind, envelope.clone()).await {
    Ok(_offset) => Ok(()),
    Err(e) => {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("Room") {
            // Best-effort create then retry once.
            let _ = self.handle.streams().create_room(room, None).await;
            self.handle.streams().publish(room, kind, envelope.clone()).await
                .map(|_| ()).map_err(|e| anyhow!("post-create publish: {e}"))
        } else {
            Err(anyhow!("synap publish: {e}"))
        }
    }
}

## When to Use

Any worker or producer that publishes to a Synap room that may not exist yet at first launch (bootstrap streams, side streams like cortex.events.invalid / cortex.events.embedded / cortex.events.graphed / cortex.events.fulltext_indexed).

## When NOT to Use

When the room genuinely should be pre-provisioned by an admin (multi-tenant boundaries, retention-tier rooms with non-default config). For those, pre-create with the right `max_events` cap.
