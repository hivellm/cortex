// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

// Phase6a §4.2 / §4.3 — Search view contract:
// - When exactly one repo is active in `filters.repo`, the
//   `postQuery` helper is called with that repo as the
//   `x-cortex-repo` header opt. Multi-valued / empty selection
//   omits the header (the daemon will return 422 and the view
//   surfaces it).
// - When the daemon answers `422 scope_repo_required`, the view
//   shows an actionable inline alert ("Scope required: …") instead
//   of leaking the raw HTTP error string.

vi.mock("../atoms/Icon", () => ({
  Icon: ({ name }: { name: string }) => <span data-testid={`icon-${name}`} />,
}));

vi.mock("../atoms/Tag", () => ({
  Tag: ({ children }: { children: React.ReactNode }) => <span>{children}</span>,
}));

const postQueryMock = vi.fn();
vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>(
    "../lib/api",
  );
  return {
    ...actual,
    postQuery: (...args: unknown[]) => postQueryMock(...args),
  };
});

// Filters context stub — switched per-test via `__filters` ref.
const filtersRef: { current: { repo?: string[] } } = { current: {} };
vi.mock("../lib/filters", () => ({
  useFilters: () => ({
    filters: filtersRef.current,
    setFilters: () => {},
    setFilter: () => {},
    clearFilters: () => {},
  }),
  hasAnyFilter: () => false,
}));

import { SearchView } from "./Search";
import { ApiError } from "../lib/api";

function withClient(node: React.ReactNode) {
  // Disable retries so 422 paths surface immediately in the test.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return <QueryClientProvider client={client}>{node}</QueryClientProvider>;
}

beforeEach(() => {
  postQueryMock.mockReset();
  filtersRef.current = {};
});

afterEach(() => {
  cleanup();
});

describe("SearchView — phase6a scope wiring", () => {
  it("forwards filters.repo[0] as opts.repo when exactly one repo is active", async () => {
    filtersRef.current = { repo: ["Cortex"] };
    postQueryMock.mockResolvedValue({
      intent: "free_search",
      query_id: "01HQUERY",
      scope_resolved: { repo: "Cortex" },
      results: { snippets: [] },
    });

    render(withClient(<SearchView />));
    fireEvent.change(screen.getByLabelText("Search query"), {
      target: { value: "retention policy" },
    });
    fireEvent.click(screen.getByRole("button", { name: /search/i }));

    await waitFor(() => expect(postQueryMock).toHaveBeenCalledTimes(1));
    const [body, opts] = postQueryMock.mock.calls[0];
    expect(body).toMatchObject({
      intent: "free_search",
      query: "retention policy",
    });
    expect(opts).toEqual({ repo: "Cortex" });
  });

  it("omits opts.repo when filters.repo is empty (daemon 422 is the right error)", async () => {
    filtersRef.current = {};
    postQueryMock.mockResolvedValue({
      intent: "free_search",
      query_id: "01HQUERY",
      scope_resolved: {},
      results: { snippets: [] },
    });

    render(withClient(<SearchView />));
    fireEvent.change(screen.getByLabelText("Search query"), {
      target: { value: "anything" },
    });
    fireEvent.click(screen.getByRole("button", { name: /search/i }));

    await waitFor(() => expect(postQueryMock).toHaveBeenCalledTimes(1));
    const [, opts] = postQueryMock.mock.calls[0];
    expect(opts).toEqual({ repo: undefined });
  });

  it("omits opts.repo when filters.repo has more than one entry (multi-repo browsing)", async () => {
    filtersRef.current = { repo: ["Cortex", "Vectorizer"] };
    postQueryMock.mockResolvedValue({
      intent: "free_search",
      query_id: "01HQUERY",
      scope_resolved: {},
      results: { snippets: [] },
    });

    render(withClient(<SearchView />));
    fireEvent.change(screen.getByLabelText("Search query"), {
      target: { value: "spanning multiple repos" },
    });
    fireEvent.click(screen.getByRole("button", { name: /search/i }));

    await waitFor(() => expect(postQueryMock).toHaveBeenCalledTimes(1));
    const [, opts] = postQueryMock.mock.calls[0];
    expect(opts).toEqual({ repo: undefined });
  });

  it("renders the inline 'Scope required' alert when the daemon answers 422 scope_repo_required", async () => {
    filtersRef.current = {};
    postQueryMock.mockRejectedValue(new ApiError(422, "422 scope_repo_required"));

    render(withClient(<SearchView />));
    fireEvent.change(screen.getByLabelText("Search query"), {
      target: { value: "ambiguous" },
    });
    fireEvent.click(screen.getByRole("button", { name: /search/i }));

    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("Scope required"),
    );
    expect(screen.getByRole("alert").textContent).toContain(
      "scope_repo_required",
    );
  });

  it("falls through to the generic error banner for non-scope failures", async () => {
    filtersRef.current = { repo: ["Cortex"] };
    postQueryMock.mockRejectedValue(new ApiError(500, "500 boom"));

    render(withClient(<SearchView />));
    fireEvent.change(screen.getByLabelText("Search query"), {
      target: { value: "trigger 500" },
    });
    fireEvent.click(screen.getByRole("button", { name: /search/i }));

    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toContain("Search failed"),
    );
  });
});
