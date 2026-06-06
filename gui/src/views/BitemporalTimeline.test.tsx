// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor, fireEvent } from "@testing-library/react";

// Phase18 §7.8 — BitemporalTimeline view smoke tests.

vi.mock("../lib/connections/useConnKey", () => ({
  useConnKey: () => "test-conn-key",
}));

const bitemporalTimelineMock = vi.fn();
const branchesMock = vi.fn();

vi.mock("../lib/api", () => {
  return {
    api: {
      bitemporalTimeline: () => bitemporalTimelineMock(),
      branches: () => branchesMock(),
    },
  };
});

import { BitemporalTimelineView } from "./BitemporalTimeline";

function renderWithQuery() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <BitemporalTimelineView />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  localStorage.removeItem("cortex.bitemporal-timeline.filters");
});

describe("BitemporalTimelineView", () => {
  it("renders timeline events on a happy-path response", async () => {
    branchesMock.mockResolvedValue({ project: "cortex", branches: [] });
    bitemporalTimelineMock.mockResolvedValue({
      project: "cortex",
      branch: "main",
      events: [
        {
          id: "ev-01",
          kind: "commit",
          title: "Initial commit",
          summary: "First commit to cortex",
          valid_from_unix: 1700000000,
          entity_id: "entity-abc",
        },
        {
          id: "ev-02",
          kind: "decision",
          title: "Adopt Rust",
          summary: "Chose Rust for the backend",
          valid_from_unix: 1700086400,
          entity_id: "entity-def",
        },
      ],
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("Initial commit")).toBeTruthy();
    });
    expect(screen.getByText("Adopt Rust")).toBeTruthy();
    // "commit" and "decision" appear twice each: once in the kind-filter chip bar
    // and once as a Tag inside the rendered event row.
    expect(screen.getAllByText("commit").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("decision").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("First commit to cortex")).toBeTruthy();
  });

  it("shows the empty-state message when no events are returned", async () => {
    branchesMock.mockResolvedValue({ project: "cortex", branches: [] });
    bitemporalTimelineMock.mockResolvedValue({
      project: "cortex",
      branch: "main",
      events: [],
    });

    renderWithQuery();

    await waitFor(() => {
      expect(
        screen.getByText("No bitemporal events found for the active filters."),
      ).toBeTruthy();
    });
  });

  it("shows unreachable message on api error", async () => {
    branchesMock.mockResolvedValue({ project: "cortex", branches: [] });
    bitemporalTimelineMock.mockRejectedValue(new Error("network error"));

    renderWithQuery();

    await waitFor(() => {
      expect(
        screen.getByText((text) => text.includes("cortex-api unreachable")),
      ).toBeTruthy();
    });
  });

  it("persists filters to localStorage when a filter changes", async () => {
    branchesMock.mockResolvedValue({ project: "cortex", branches: [] });
    bitemporalTimelineMock.mockResolvedValue({
      project: "cortex",
      branch: "main",
      events: [],
    });

    renderWithQuery();

    // Wait for initial render
    await waitFor(() => {
      expect(screen.getByText("No bitemporal events found for the active filters.")).toBeTruthy();
    });

    // Click the "commit" kind chip to toggle it
    const commitBtn = screen.getByTitle("Filter to kind: commit");
    fireEvent.click(commitBtn);

    // The filter should be written to localStorage
    await waitFor(() => {
      const stored = localStorage.getItem("cortex.bitemporal-timeline.filters");
      expect(stored).not.toBeNull();
      const parsed = JSON.parse(stored!);
      expect(parsed.kinds).toContain("commit");
    });
  });
});
