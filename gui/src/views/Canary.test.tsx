// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";

// Phase15f §4.2 §3.4 — CanaryView smoke tests. Mock the api module so
// the view renders deterministically without touching the network.

vi.mock("../lib/api", () => ({
  api: {
    dashboardCanary: vi.fn().mockResolvedValue({
      runs: [
        {
          ts: "2026-06-09T12:03:00Z",
          status: "error",
          latency_ms: null,
          error_message: "http 503",
        },
        {
          ts: "2026-06-09T12:02:00Z",
          status: "error",
          latency_ms: null,
          error_message: "http 500",
        },
        {
          ts: "2026-06-09T12:01:00Z",
          status: "ok",
          latency_ms: 12,
          error_message: null,
        },
      ],
      last_failure: {
        ts: "2026-06-09T12:03:00Z",
        status: "error",
        latency_ms: null,
        error_message: "http 503",
      },
    }),
  },
}));

vi.mock("../lib/connections/useConnKey", () => ({
  useConnKey: () => "test-conn",
}));

import { CanaryView } from "./Canary";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function withQuery(ui: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return <QueryClientProvider client={client}>{ui}</QueryClientProvider>;
}

describe("CanaryView", () => {
  it("renders the Canary heading", async () => {
    render(withQuery(<CanaryView />));
    expect(await screen.findByRole("heading", { name: /canary/i })).toBeInTheDocument();
  });

  it("renders alarm pill when last_failure is set", async () => {
    render(withQuery(<CanaryView />));
    expect(await screen.findByText("alarm")).toBeInTheDocument();
  });

  it("renders the failure banner with the error_message", async () => {
    render(withQuery(<CanaryView />));
    // "http 503" appears in both the failure banner and the sparkline table row.
    const els = await screen.findAllByText(/http 503/);
    expect(els.length).toBeGreaterThan(0);
  });

  it("renders the run count in the sparkline summary", async () => {
    render(withQuery(<CanaryView />));
    // 3 ticks: 1 ok, 2 errors. The summary paragraph contains all counts.
    // Use findAllByText to tolerate multiple container elements that include
    // this text as part of their content.
    const tickEls = await screen.findAllByText(/3 ticks/);
    expect(tickEls.length).toBeGreaterThan(0);
    const errEls = screen.getAllByText(/2 error/);
    expect(errEls.length).toBeGreaterThan(0);
  });

  it("renders a row per run in the sparkline table", async () => {
    render(withQuery(<CanaryView />));
    // All three ts values must appear somewhere in the rendered DOM.
    // Use findAllByText to tolerate the timestamp appearing in both the
    // failure banner and the sparkline table row.
    const ts1 = await screen.findAllByText("2026-06-09T12:03:00Z");
    expect(ts1.length).toBeGreaterThan(0);
    const ts2 = screen.getAllByText("2026-06-09T12:02:00Z");
    expect(ts2.length).toBeGreaterThan(0);
    const ts3 = screen.getAllByText("2026-06-09T12:01:00Z");
    expect(ts3.length).toBeGreaterThan(0);
  });
});
