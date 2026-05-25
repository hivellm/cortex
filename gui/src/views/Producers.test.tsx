// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

// Phase13f §5.5 — Producers view smoke tests. Mock the dashboard
// `/v1/dashboard/producers` endpoint so the view renders
// deterministically without hitting the daemon.

vi.mock("../lib/connections/useConnKey", () => ({
  useConnKey: () => "test-conn-key",
}));

const producersMock = vi.fn();

vi.mock("../lib/api", () => {
  return {
    api: {
      producers: () => producersMock(),
    },
  };
});

import { ProducersView } from "./Producers";

function renderWithQuery() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ProducersView />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ProducersView", () => {
  it("renders rows sorted by producer and scope from the report view", async () => {
    producersMock.mockResolvedValue({
      total: 2,
      rows: [
        {
          producer_name: "bootstrap",
          scope: "cortex",
          last_event_id: "01A",
          last_occurred_at: "2026-05-25T12:00:00Z",
          accumulated_at: "2026-05-25T12:00:01Z",
          has_progress: true,
        },
        {
          producer_name: "claude_archive",
          scope: "proj1",
          last_event_id: null,
          last_occurred_at: "2026-05-25T12:00:00Z",
          accumulated_at: "2026-05-25T12:00:02Z",
          has_progress: false,
        },
      ],
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("2 (producer, scope) pairs.")).toBeTruthy();
    });
    expect(screen.getByText("bootstrap")).toBeTruthy();
    expect(screen.getByText("claude_archive")).toBeTruthy();
    expect(screen.getByText("01A")).toBeTruthy();
    // has_progress pill shows yes/no
    const pills = screen.getAllByText(/^(yes|no)$/);
    expect(pills.length).toBe(2);
  });

  it("surfaces empty state when no checkpoints recorded", async () => {
    producersMock.mockResolvedValue({
      total: 0,
      rows: [],
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("No producer checkpoints recorded.")).toBeTruthy();
    });
  });

  it("uses singular phrasing for total = 1", async () => {
    producersMock.mockResolvedValue({
      total: 1,
      rows: [
        {
          producer_name: "topic_cards",
          scope: "topic-slug",
          last_event_id: "01T",
          last_occurred_at: "2026-05-25T12:00:00Z",
          accumulated_at: "2026-05-25T12:00:01Z",
          has_progress: true,
        },
      ],
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("1 (producer, scope) pair.")).toBeTruthy();
    });
  });
});
