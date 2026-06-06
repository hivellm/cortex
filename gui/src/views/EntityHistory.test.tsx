// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor, fireEvent } from "@testing-library/react";

// Phase18 §7.8 — EntityHistory view smoke tests.

vi.mock("../lib/connections/useConnKey", () => ({
  useConnKey: () => "test-conn-key",
}));

const entityHistoryMock = vi.fn();
const entitySupersessionMock = vi.fn();

vi.mock("../lib/api", () => {
  return {
    api: {
      entityHistory: () => entityHistoryMock(),
      entitySupersession: () => entitySupersessionMock(),
    },
  };
});

import { EntityHistoryView } from "./EntityHistory";

function renderWithQuery() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <EntityHistoryView />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("EntityHistoryView", () => {
  it("renders history events and supersession lineage on a happy-path response", async () => {
    entityHistoryMock.mockResolvedValue({
      entity_id: "entity-abc",
      events: [
        {
          id: "ev-01",
          kind: "decision",
          title: "Adopt Rust",
          summary: "Chosen for backend",
          valid_from_unix: 1700000000,
          recorded_at_unix: 1700000050,
        },
        {
          id: "ev-02",
          kind: "learning",
          title: "Rust async patterns",
          summary: "tokio preferred",
          valid_from_unix: 1700086400,
          recorded_at_unix: 1700086420,
        },
      ],
    });
    entitySupersessionMock.mockResolvedValue({
      entity_id: "entity-abc",
      lineage: {
        id: "entity-abc",
        label: "Adopt Rust",
        predecessors: [{ id: "entity-old", kind: "decision" }],
        successors: [],
      },
    });

    renderWithQuery();

    // Simulate loading an entity
    const input = screen.getByLabelText("Entity ID");
    fireEvent.change(input, { target: { value: "entity-abc" } });
    const loadBtn = screen.getByText("Load");
    fireEvent.click(loadBtn);

    await waitFor(() => {
      // "Adopt Rust" appears twice: once in the lineage panel (label) and once
      // in the event row title — both are correct renderings.
      expect(screen.getAllByText("Adopt Rust").length).toBeGreaterThanOrEqual(1);
    });
    expect(screen.getByText("Rust async patterns")).toBeTruthy();
    expect(screen.getByText("Chosen for backend")).toBeTruthy();
    expect(screen.getByText("Supersession Lineage")).toBeTruthy();
    // Predecessor node should be visible
    expect(screen.getByText("entity-old")).toBeTruthy();
  });

  it("shows the empty-state message when no history events are returned", async () => {
    entityHistoryMock.mockResolvedValue({ entity_id: "entity-empty", events: [] });
    entitySupersessionMock.mockResolvedValue({
      entity_id: "entity-empty",
      lineage: undefined,
    });

    renderWithQuery();

    const input = screen.getByLabelText("Entity ID");
    fireEvent.change(input, { target: { value: "entity-empty" } });
    fireEvent.click(screen.getByText("Load"));

    await waitFor(() => {
      expect(
        screen.getByText("No history events found for this entity."),
      ).toBeTruthy();
    });
  });

  it("shows unreachable message on api error", async () => {
    entityHistoryMock.mockRejectedValue(new Error("network error"));
    entitySupersessionMock.mockRejectedValue(new Error("network error"));

    renderWithQuery();

    const input = screen.getByLabelText("Entity ID");
    fireEvent.change(input, { target: { value: "entity-bad" } });
    fireEvent.click(screen.getByText("Load"));

    await waitFor(() => {
      expect(
        screen.getByText((text) => text.includes("cortex-api unreachable")),
      ).toBeTruthy();
    });
  });

  it("pre-fills the entity ID when a cortex:open-entity-history event fires", async () => {
    entityHistoryMock.mockResolvedValue({
      entity_id: "entity-from-event",
      events: [
        {
          id: "ev-x",
          kind: "commit",
          title: "Event-driven load",
          valid_from_unix: 1700000000,
        },
      ],
    });
    entitySupersessionMock.mockResolvedValue({
      entity_id: "entity-from-event",
      lineage: undefined,
    });

    renderWithQuery();

    // Fire the cross-view event
    window.dispatchEvent(
      new CustomEvent("cortex:open-entity-history", {
        detail: { id: "entity-from-event" },
      }),
    );

    await waitFor(() => {
      expect(screen.getByText("Event-driven load")).toBeTruthy();
    });
    const input = screen.getByLabelText("Entity ID") as HTMLInputElement;
    expect(input.value).toBe("entity-from-event");
  });
});
