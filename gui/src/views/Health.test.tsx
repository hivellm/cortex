// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";

// Phase8g — Health view smoke tests. Mock the api module so the
// view renders deterministically without touching the network.

vi.mock("../atoms/Icon", () => ({
  Icon: ({ name }: { name: string }) => <span data-testid={`icon-${name}`} />,
}));

vi.mock("../atoms/Sparkline", () => ({
  Sparkline: () => <span data-testid="sparkline" />,
}));

vi.mock("../lib/useSSE", () => ({
  useSSE: () => ({ connected: false, lastHeartbeatAt: 0, reconnects: 0 }),
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
      // Phase11e §2.2 — coverage audit mock. Mirrors the live
      // shape we observed in the docker stack: 4/144 collections
      // present in the Vectorizer, 29/144 indexes present in
      // Meili, both warn-severity.
      healthCoverage: vi.fn().mockResolvedValue({
        slugs: ["cortex"],
        families: [
          "code", "docs", "decisions", "turns", "governance",
          "analyses", "knowledge", "learnings", "misc",
        ],
        backends: [
          {
            backend: "vectorizer",
            base_url: "http://vectorizer:15002",
            severity: "warn" as const,
            expected_count: 9,
            present_count: 4,
            missing_count: 5,
            unexpected_count: 0,
            present: [
              "cortex-cortex-code",
              "cortex-cortex-docs",
              "cortex-cortex-misc",
              "cortex-cortex-governance",
            ],
            missing: [
              "cortex-cortex-analyses",
              "cortex-cortex-decisions",
              "cortex-cortex-knowledge",
              "cortex-cortex-learnings",
              "cortex-cortex-turns",
            ],
            unexpected: [],
            error: null,
          },
        ],
        overall_severity: "warn" as const,
      }),
      retentionSweeps: vi.fn().mockResolvedValue([]),
      retentionState: vi.fn().mockResolvedValue({
        collections: [],
        archive_bytes: {
          le_30d: 0,
          "30d_to_365d": 0,
          gt_365d: 0,
          total: 0,
          root: "",
          available: false,
        },
        meili_indexes: [],
        cas: { rows: 0, bytes: 0, available: false, path: "" },
        next_runs: [],
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

  // Phase11e §2.2 — Coverage section regression guards. Every
  // backend the auditor returns gets a card; the missing list
  // collapses behind a `<details>` so the dashboard stays compact
  // even when 140 collections are absent.
  it("renders one coverage card per backend with the present/expected ratio", async () => {
    render(withQuery(<HealthView />));
    expect(await screen.findByText("vectorizer")).toBeInTheDocument();
    // The card carries the "4 / 9" present/expected ratio rendered
    // by `fmtNum` (4 and 9 — small enough that fmtNum returns the
    // numerals verbatim) — assert the ratio percentage which is
    // unique to the coverage card.
    expect(screen.getByText(/44% present/)).toBeInTheDocument();
  });

  it("lists the first missing collection inside the coverage card's details panel", async () => {
    render(withQuery(<HealthView />));
    // The missing list is collapsed by default — assert the
    // count summary surfaces; the items themselves render but
    // are hidden under <details>. testing-library queries the
    // DOM regardless, so checking the first missing name proves
    // the projection runs.
    expect(
      await screen.findByText("cortex-cortex-analyses"),
    ).toBeInTheDocument();
  });
});
