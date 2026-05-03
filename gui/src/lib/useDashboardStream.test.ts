// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { QueryClient } from "@tanstack/react-query";

import {
  DASHBOARD_EVENT_KINDS,
  invalidateAllDashboardQueries,
} from "./useDashboardStream";

describe("DASHBOARD_EVENT_KINDS", () => {
  it("matches the spec 21 envelope tags exactly", () => {
    // Tags are part of the wire contract — the server emits them as
    // `event:` field values and the GUI listens by name. Drift between
    // client and server here silently drops events. Lock the list.
    expect(DASHBOARD_EVENT_KINDS).toEqual([
      "task.changed",
      "handoff.appended",
      "decision.changed",
      "memory.appended",
      "knowledge.added",
      "learning.added",
    ]);
  });
});

describe("invalidateAllDashboardQueries", () => {
  let client: QueryClient;
  let calls: Array<readonly unknown[]>;

  beforeEach(() => {
    client = new QueryClient();
    calls = [];
    // Patch the method directly so the recorded calls capture the
    // exact `queryKey` we pass into the helper. `vi.spyOn` widens
    // the parameter type to `unknown[]`, which TanStack's typed
    // `invalidateQueries` rejects under strict TS.
    const original = client.invalidateQueries.bind(client);
    client.invalidateQueries = ((filters?: { queryKey?: readonly unknown[] }) => {
      if (filters?.queryKey) calls.push(filters.queryKey);
      return original(filters);
    }) as typeof client.invalidateQueries;
  });

  afterEach(() => {
    client.clear();
  });

  it("fires one invalidate per dashboard key prefix", () => {
    invalidateAllDashboardQueries(client, "test-conn");
    // 6 kinds: tasks emits 2 prefixes (tasks + tasks-summary), learning
    // emits 2 (learnings + memory), the rest emit 1 each = 8 total.
    expect(calls).toHaveLength(8);
    expect(calls).toEqual(
      expect.arrayContaining([
        ["test-conn", "tasks"],
        ["test-conn", "tasks-summary"],
        ["test-conn", "handoffs"],
        ["test-conn", "decisions"],
        ["test-conn", "memory"],
        ["test-conn", "knowledge"],
        ["test-conn", "learnings"],
      ]),
    );
  });

  it("scopes every invalidation to the active connKey", () => {
    invalidateAllDashboardQueries(client, "alpha");
    const heads = calls.map((k) => k[0]);
    expect(new Set(heads)).toEqual(new Set(["alpha"]));
  });
});
