// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

// Phase14f §4.4 — Pre-Thinking Quality view snapshot test. Mocks
// the `/v1/health/pre-thinking` endpoint so the view renders
// deterministically without hitting the daemon.

vi.mock("../lib/connections/useConnKey", () => ({
  useConnKey: () => "test-conn-key",
}));

const preThinkingMock = vi.fn();

vi.mock("../lib/api", () => {
  return {
    api: {
      preThinkingQuality: () => preThinkingMock(),
    },
  };
});

import { PreThinkingQualityView } from "./PreThinkingQuality";

function renderWithQuery() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <PreThinkingQualityView />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("PreThinkingQualityView", () => {
  it("renders breaker banner + per-intent tables", async () => {
    preThinkingMock.mockResolvedValueOnce({
      breaker_state: "closed",
      failures_in_window: 0,
      fail_open_total: { timeout: 2, breaker_open: 1 },
      fail_open_sum: 3,
      bundle_bytes_per_intent: {
        explain: { count: 10, p50: 12000, p95: 22000, p99: 24000 },
        law_check: { count: 5, p50: 4000, p95: 9000, p99: 11000 },
      },
      helpful_rate_per_intent: {
        explain: { helpful: 7, unhelpful: 3, rate: 0.7 },
        law_check: { helpful: 4, unhelpful: 0, rate: 1.0 },
      },
    });

    renderWithQuery();

    await waitFor(() =>
      expect(screen.getByTestId("pt-breaker-banner")).toBeTruthy(),
    );
    expect(screen.getByTestId("pt-bytes-table")).toBeTruthy();
    expect(screen.getByTestId("pt-helpful-table")).toBeTruthy();
    // Banner reason counters surface.
    expect(screen.getByText(/timeout=2/)).toBeTruthy();
    expect(screen.getByText(/breaker_open=1/)).toBeTruthy();
    // p50/p95/p99 visible.
    expect(screen.getByText("12000")).toBeTruthy();
    expect(screen.getByText("22000")).toBeTruthy();
    // Helpful rate percent-formatted.
    expect(screen.getByText("70.0%")).toBeTruthy();
    expect(screen.getByText("100.0%")).toBeTruthy();
  });

  it("renders empty-state sections when no samples / feedback", async () => {
    preThinkingMock.mockResolvedValueOnce({
      breaker_state: "closed",
      failures_in_window: 0,
      fail_open_total: {},
      fail_open_sum: 0,
      bundle_bytes_per_intent: {},
      helpful_rate_per_intent: {},
    });

    renderWithQuery();

    await waitFor(() =>
      expect(screen.getByTestId("pt-breaker-banner")).toBeTruthy(),
    );
    expect(
      screen.getByText(
        /No samples yet — pre-thinking has not run since boot\./,
      ),
    ).toBeTruthy();
    expect(
      screen.getByText(
        /No feedback recorded yet — POST \/v1\/pre-thinking\/feedback to seed\./,
      ),
    ).toBeTruthy();
  });

  it("renders breaker open state with warn colour cue", async () => {
    preThinkingMock.mockResolvedValueOnce({
      breaker_state: "open",
      failures_in_window: 5,
      fail_open_total: { timeout: 5 },
      fail_open_sum: 5,
      bundle_bytes_per_intent: {},
      helpful_rate_per_intent: {},
    });

    renderWithQuery();

    await waitFor(() =>
      expect(screen.getByTestId("pt-breaker-banner")).toBeTruthy(),
    );
    expect(screen.getByText("open")).toBeTruthy();
    expect(screen.getByText(/failures_in_window: 5/)).toBeTruthy();
  });
});
