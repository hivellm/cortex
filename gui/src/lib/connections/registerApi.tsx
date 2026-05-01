/**
 * Register the connections store with the api.ts fetcher layer —
 * phase3 §3.4.
 *
 * This component lives just inside `<ConnectionsProvider>` and
 * wires the api.ts module-level `activeConnectionResolver` to read
 * from the store on every fetch. We can't do this from
 * `ConnectionsProvider` directly because that would import api.ts
 * (which already imports the connection types) and form a circular
 * dependency at module-load time. Splitting the registration into
 * a child component keeps the dep graph linear.
 */

import { useEffect, useRef } from "react";

import { useConnections } from "./store";
import {
  setActiveConnectionResolver,
  type ApiConnection,
} from "../api";

function projectForApi(c: ReturnType<typeof useConnections>["active"]): ApiConnection {
  return {
    id: c.id,
    baseUrl: c.baseUrl,
    auth: c.auth,
  };
}

export function ApiResolverBinding({ children }: { children: React.ReactNode }) {
  const ctx = useConnections();
  // Keep a ref so the resolver function stays stable across renders
  // — we don't want to re-bind the api.ts pointer every reconcile,
  // and the resolver always reads the latest active connection from
  // the ref's current value.
  const activeRef = useRef(ctx.active);
  activeRef.current = ctx.active;

  useEffect(() => {
    setActiveConnectionResolver(() => projectForApi(activeRef.current));
    // The resolver function is module-singleton-scoped on api.ts; we
    // do not need to clean up on unmount because there is exactly
    // one ApiResolverBinding tree at any time. If a future test
    // setup mounts two providers, the latest mount wins — same
    // behaviour as the legacy single-backend code.
  }, []);

  return <>{children}</>;
}
