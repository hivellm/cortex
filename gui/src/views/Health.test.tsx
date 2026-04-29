// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";

// Phase8g — Health view smoke tests. Mock the api module so the
// view renders deterministically without touching the network.

vi.mock("../atoms/Icon", () => ({
  Icon: ({ name }: { name: string }) => <span data-testid={`icon-${name}`} />,
}));

vi.mock("../lib/api", () => {
  return {
    api: {
      healthOverview: vi.fn().mockResolvedValue({
        overall: "degraded" as const,
        subsystems: [
          {
            name: "cortex-adapter",
            state: "down" as const,
            latency_ms: 0,
            last_error: "ipc pipe not alive",
            version: "0.1.0",
            since: "2026-04-29T10:00:00Z",
            extras: {},
          },
          {
            name: "cortex-api",
            state: "ok" as const,
            latency_ms: 5,
            version: "0.1.0",
            since: "2026-04-29T10:00:00Z",
            extras: {},
          },
        ],
        checked_at: "2026-04-29T18:00:00Z",
      }),
      healthFreshness: vi.fn().mockResolvedValue([
        {
          key: "adapter.last_frame.PostToolUse",
          last_event_ts_ms: 1_000,
          gap_seconds: 90,
          severity: "warn" as const,
        },
      ]),
      healthDivergence: vi.fn().mockResolvedValue([
        {
          pair: "adapter.envelopes_built -> adapter.envelopes_publish_ok",
          upstream: 100,
          downstream: 20,
          delta: 80,
          delta_growth: 80,
          severity: "critical" as const,
        },
      ]),
      healthVersions: vi.fn().mockResolvedValue({
        head_sha: "abc1234567890",
        head_sha_short: "abc1234",
        running_binaries: [],
        drift: [],
        all_in_sync: true,
      }),
      healthConfig: vi.fn().mockResolvedValue({
        findings: [
          { severity: "ok" as const, source: ".env", message: "loaded" },
          {
            severity: "critical" as const,
            source: "cross-check",
            message: "adapter.toml.endpoint mismatch",
          },
        ],
        surfaces_read: 4,
      }),
    },
  };
});

import { HealthView } from "./Health";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function withQuery(ui: React.ReactNode) {
  // Disable retries + cache between tests so each render starts
  // from a cold state.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
  return <QueryClientProvider client={client}>{ui}</QueryClientProvider>;
}

describe("HealthView", () => {
  it("renders the overall banner driven by /v1/health.overall", async () => {
    render(withQuery(<HealthView />));
    expect(await screen.findByText(/subsystem.*degraded/)).toBeInTheDocument();
  });

  it("renders one subsystem card per /v1/health.subsystems[] entry", async () => {
    render(withQuery(<HealthView />));
    expect(await screen.findByText("cortex-adapter")).toBeInTheDocument();
    expect(screen.getByText("cortex-api")).toBeInTheDocument();
  });

  it("renders the divergence row when severity != ok", async () => {
    render(withQuery(<HealthView />));
    expect(
      await screen.findByText(/adapter\.envelopes_built/),
    ).toBeInTheDocument();
  });

  it("filters config audit findings to severity != ok", async () => {
    render(withQuery(<HealthView />));
    // The critical finding shows; the ok one is filtered out (it
    // would otherwise drown the audit table on healthy stacks).
    expect(
      await screen.findByText("adapter.toml.endpoint mismatch"),
    ).toBeInTheDocument();
    expect(screen.queryByText("loaded")).toBeNull();
  });

  it("sorts freshness rows by gap_seconds desc and renders the gap label", async () => {
    render(withQuery(<HealthView />));
    expect(await screen.findByText(/90s/)).toBeInTheDocument();
  });
});
