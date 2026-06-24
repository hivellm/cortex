// @vitest-environment jsdom
// Phase21 §8.2 — AclStatsView smoke tests (happy / empty / error).

import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

vi.mock("../lib/connections/useConnKey", () => ({
  useConnKey: () => "test-conn-key",
}));

const aclStatsMock = vi.fn();

vi.mock("../lib/api", () => ({
  api: {
    aclStats: () => aclStatsMock(),
  },
}));

import { AclStatsView } from "./AclStats";

function renderWithQuery() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <AclStatsView />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("AclStatsView", () => {
  it("renders totals, distribution, and top-denied-principals on happy path", async () => {
    aclStatsMock.mockResolvedValue({
      total_evaluated: 120,
      total_granted: 95,
      total_denied: 25,
      denial_rate_over_time: [],
      classification_distribution: [
        { repo: "cortex", level: 0, count: 60 },
        { repo: "cortex", level: 2, count: 10 },
      ],
      top_denied_principals: [
        { principal_id: "alice", denial_count: 20 },
        { principal_id: "bob", denial_count: 5 },
      ],
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("120")).toBeTruthy();
    });
    expect(screen.getByText("95")).toBeTruthy();
    expect(screen.getByText("25")).toBeTruthy();

    // Distribution table (two rows for "cortex", so use getAllByText)
    expect(screen.getAllByText("cortex").length).toBe(2);
    expect(screen.getByText("public")).toBeTruthy();
    expect(screen.getByText("confidential")).toBeTruthy();

    // Top denied principals
    expect(screen.getByText("alice")).toBeTruthy();
    expect(screen.getByText("bob")).toBeTruthy();
    expect(screen.getByText("20")).toBeTruthy();
  });

  it("shows no-data notice when total_evaluated is 0 (AC disabled)", async () => {
    aclStatsMock.mockResolvedValue({
      total_evaluated: 0,
      total_granted: 0,
      total_denied: 0,
      denial_rate_over_time: [],
      classification_distribution: [],
      top_denied_principals: [],
    });

    renderWithQuery();

    await waitFor(() => {
      expect(
        screen.getByText("No access decisions recorded — AC may be disabled."),
      ).toBeTruthy();
    });
    expect(screen.getByText("No classified hits in the keyword lane.")).toBeTruthy();
    expect(screen.getByText("No denials recorded.")).toBeTruthy();
  });

  it("renders error state when the endpoint fails", async () => {
    aclStatsMock.mockRejectedValue(new Error("network error"));

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("Failed to load ACL stats.")).toBeTruthy();
    });
  });
});
