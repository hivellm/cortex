// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";

// Phase9i — Retention view smoke tests. Mock api + SSE so the view
// renders deterministically without hitting the network.

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
      retentionSweeps: vi.fn().mockResolvedValue([
        // Two consecutive failures for parquet_rollup → banner MUST appear.
        {
          sweep_id: "01ROLLA",
          started_at: new Date(Date.now() - 60_000).toISOString(),
          finished_at: new Date(Date.now() - 30_000).toISOString(),
          status: "failed",
          records_demoted: 0,
          records_dropped: 0,
          stages: { parquet_rollup: { error: "S3 timeout" } },
        },
        {
          sweep_id: "01ROLLB",
          started_at: new Date(Date.now() - 86_400_000).toISOString(),
          finished_at: new Date(Date.now() - 86_400_000 + 5_000).toISOString(),
          status: "failed",
          records_demoted: 0,
          records_dropped: 0,
          stages: { parquet_rollup: { error: "S3 timeout" } },
        },
        // One healthy tier sweep with a reclamation counter.
        {
          sweep_id: "01TIER",
          started_at: new Date(Date.now() - 3_600_000).toISOString(),
          finished_at: new Date(Date.now() - 3_590_000).toISOString(),
          status: "success",
          records_demoted: 5,
          records_dropped: 0,
          stages: { tier_sweep: { bytes_reclaimed: 4096 } },
        },
      ]),
      retentionState: vi.fn().mockResolvedValue({
        collections: [],
        archive_bytes: {
          le_30d: 1024,
          "30d_to_365d": 0,
          gt_365d: 0,
          total: 1024,
          root: "/tmp/archive",
          available: true,
        },
        meili_indexes: [],
        cas: { rows: 5, bytes: 2048, available: true, path: "/tmp/cas.sqlite" },
        next_runs: [
          // phase11v §1.6 — schedule lane is the source of truth for
          // last/next-run. tier_sweep is fresh + green; consolidation_prune
          // is failing; memory_consolidate is operator-disabled.
          {
            sweep: "tier_sweep",
            next_run: new Date(Date.now() + 3_600_000).toISOString(),
            last_run: new Date(Date.now() - 1_800_000).toISOString(),
            last_status: "success",
          },
          { sweep: "parquet_rollup", next_run: "never" },
          {
            sweep: "consolidation_prune",
            next_run: new Date(Date.now() + 86_400_000).toISOString(),
            last_run: new Date(Date.now() - 600_000).toISOString(),
            last_status: "failed",
            failure_streak: 1,
          },
          { sweep: "memory_consolidate", next_run: "disabled" },
        ],
      }),
    },
  };
});

import { RetentionView } from "./Retention";

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

describe("RetentionView", () => {
  it("renders the title", async () => {
    render(withQuery(<RetentionView />));
    expect(
      await screen.findByRole("heading", { level: 1, name: /Retention/i }),
    ).toBeInTheDocument();
  });

  it("renders one card per sweep type", async () => {
    render(withQuery(<RetentionView />));
    expect(await screen.findByText("Tier sweep")).toBeInTheDocument();
    expect(screen.getByText("Archive rollup")).toBeInTheDocument();
    expect(screen.getByText("CAS vacuum")).toBeInTheDocument();
    expect(screen.getByText("PII enforcement")).toBeInTheDocument();
    expect(screen.getByText("Turn digest")).toBeInTheDocument();
    expect(screen.getByText("Meili prune")).toBeInTheDocument();
    expect(screen.getByText("Metadata reap")).toBeInTheDocument();
    // phase11v §1.6 — three new sweep cards appear after the
    // schedule-lane wiring lands.
    expect(screen.getByText("Consolidator (nightly)")).toBeInTheDocument();
    expect(screen.getByText("Consolidation prune")).toBeInTheDocument();
    expect(screen.getByText("Memory consolidate")).toBeInTheDocument();
  });

  it("renders the disabled pill for memory_consolidate when next_run is 'disabled'", async () => {
    render(withQuery(<RetentionView />));
    // Pill text only appears after the schedule lane resolves.
    expect(await screen.findByText("disabled")).toBeInTheDocument();
  });

  it("renders the failure_streak badge on consolidation_prune", async () => {
    render(withQuery(<RetentionView />));
    // Status badge appears once both queries resolve. The `(×1)` is
    // the consecutive-failure marker.
    expect(
      await screen.findByText((content) => /failed\s*\(×1\)/i.test(content)),
    ).toBeInTheDocument();
  });

  it("shows the failure banner when a sweep type has 2 consecutive failures", async () => {
    render(withQuery(<RetentionView />));
    // Banner copy mentions the sweep label + the failure count.
    expect(await screen.findByText(/Sweep failing/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Archive rollup .*2 consecutive failures/i),
    ).toBeInTheDocument();
    expect(screen.getByText(/last_error: S3 timeout/)).toBeInTheDocument();
  });

  it("renders the storage breakdown with archive + cas rows", async () => {
    render(withQuery(<RetentionView />));
    expect(await screen.findByText(/archive\/le_30d/)).toBeInTheDocument();
    expect(screen.getByText(/cas\/blobs/)).toBeInTheDocument();
  });

  it("starts with an empty live log message", async () => {
    render(withQuery(<RetentionView />));
    expect(
      await screen.findByText(/Waiting for retention events/i),
    ).toBeInTheDocument();
  });
});
