// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

// Phase13f §5.5 — Coverage view smoke tests. Mock the dashboard
// `/v1/dashboard/coverage` endpoint so the view renders
// deterministically without hitting the daemon.

vi.mock("../lib/connections/useConnKey", () => ({
  useConnKey: () => "test-conn-key",
}));

const coverageMock = vi.fn();

vi.mock("../lib/api", () => {
  return {
    api: {
      coverage: () => coverageMock(),
    },
  };
});

import { CoverageView } from "./Coverage";

function renderWithQuery() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <CoverageView />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("CoverageView", () => {
  it("renders the four backend rows in fixed order on a healthy scan", async () => {
    coverageMock.mockResolvedValue({
      metadata_db: "/var/lib/cortex/metadata.sqlite",
      rows_total: 1024,
      backends: [
        { backend: "nexus", missing: 0, sample_event_ids: [] },
        { backend: "vectorizer", missing: 0, sample_event_ids: [] },
        { backend: "meili", missing: 0, sample_event_ids: [] },
        { backend: "archive", missing: 0, sample_event_ids: [] },
      ],
      is_healthy: true,
      latency_ms: 42,
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("Healthy")).toBeTruthy();
    });
    // React splits interpolations into separate text nodes; assert
    // each fragment lands on the same element. The number uses
    // `toLocaleString()` whose output depends on the runtime locale,
    // so the test re-derives the expected string the same way.
    const expectedCount = (1024).toLocaleString();
    const muted = screen.getByText((_content, el) =>
      Boolean(
        el?.classList.contains("view__muted") &&
          el?.textContent?.includes("Scanned") &&
          el?.textContent?.includes(expectedCount) &&
          el?.textContent?.includes("42 ms"),
      ),
    );
    expect(muted).toBeTruthy();
    expect(screen.getByText("nexus")).toBeTruthy();
    expect(screen.getByText("vectorizer")).toBeTruthy();
    expect(screen.getByText("meili")).toBeTruthy();
    expect(screen.getByText("archive")).toBeTruthy();
  });

  it("surfaces 'not wired' when metadata_db is empty", async () => {
    coverageMock.mockResolvedValue({
      metadata_db: "",
      rows_total: 0,
      backends: [
        { backend: "nexus", missing: 0, sample_event_ids: [] },
        { backend: "vectorizer", missing: 0, sample_event_ids: [] },
        { backend: "meili", missing: 0, sample_event_ids: [] },
        { backend: "archive", missing: 0, sample_event_ids: [] },
      ],
      is_healthy: true,
      latency_ms: 0,
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("Metadata store not wired.")).toBeTruthy();
    });
  });

  it("flags coverage gaps when is_healthy is false", async () => {
    coverageMock.mockResolvedValue({
      metadata_db: "/db",
      rows_total: 100,
      backends: [
        {
          backend: "nexus",
          missing: 3,
          sample_event_ids: ["01A", "01B", "01C"],
        },
        { backend: "vectorizer", missing: 0, sample_event_ids: [] },
        { backend: "meili", missing: 0, sample_event_ids: [] },
        { backend: "archive", missing: 0, sample_event_ids: [] },
      ],
      is_healthy: false,
      latency_ms: 7,
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("Coverage gaps detected")).toBeTruthy();
    });
    expect(screen.getByText((3).toLocaleString())).toBeTruthy();
    expect(screen.getByText("01A, 01B, 01C")).toBeTruthy();
  });
});
