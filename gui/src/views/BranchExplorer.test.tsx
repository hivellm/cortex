// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";

// Phase18 §7.8 — BranchExplorer view smoke tests.

vi.mock("../lib/connections/useConnKey", () => ({
  useConnKey: () => "test-conn-key",
}));

const branchesMock = vi.fn();
const branchDetailMock = vi.fn();

vi.mock("../lib/api", () => {
  return {
    api: {
      branches: () => branchesMock(),
      branchDetail: () => branchDetailMock(),
    },
  };
});

import { BranchExplorerView } from "./BranchExplorer";

function renderWithQuery() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <BranchExplorerView />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("BranchExplorerView", () => {
  it("renders branch names with status badges on a happy-path response", async () => {
    branchesMock.mockResolvedValue({
      project: "cortex",
      branches: [
        {
          id: "branch-main",
          name: "main",
          parent_branch_id: null,
          status: "active",
          fork_valid_time: "2024-01-01T00:00:00Z",
        },
        {
          id: "branch-feat",
          name: "feat/timeline",
          parent_branch_id: "branch-main",
          status: "merged",
          merge_strategy: "fast-forward",
        },
      ],
    });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("main")).toBeTruthy();
    });
    expect(screen.getByText("feat/timeline")).toBeTruthy();
    expect(screen.getByText("active")).toBeTruthy();
    expect(screen.getByText("merged")).toBeTruthy();
  });

  it("shows the empty-state message when no branches are returned", async () => {
    branchesMock.mockResolvedValue({ project: "cortex", branches: [] });

    renderWithQuery();

    await waitFor(() => {
      expect(screen.getByText("No branches found for this project.")).toBeTruthy();
    });
  });

  it("shows unreachable message on api error", async () => {
    branchesMock.mockRejectedValue(new Error("network error"));

    renderWithQuery();

    await waitFor(() => {
      expect(
        screen.getByText((text) => text.includes("cortex-api unreachable")),
      ).toBeTruthy();
    });
  });
});
