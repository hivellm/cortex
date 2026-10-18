import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LocalWorkspace } from "./LocalWorkspace";
import { localRequest } from "./api";
import type { LocalRecord } from "./types";
vi.mock("./api", () => ({ localRequest: vi.fn() }));
const api = vi.mocked(localRequest);
const record: LocalRecord = { id: "m1", repository: "Cortex", revision: 1, kind: "note", authority: "observation", title: "Preserved memory", body: "<script>unsafe()</script>", source: { kind: "test", locator: "fixture" }, links: [], metadata: {}, created_at: "2026-10-18", updated_at: "2026-10-18", content_sha256: "abc" };
const reference = { id: "m1", repository: "Cortex", revision: 1, content_sha256: "abc", source_kind: "test", source_sha256: "def", body_clipped: false };
const delivery = { id: "delivery-1", repository: "Cortex", recorded_at: "2026-10-18", evidence_label: "prepared_delivery", payload: { type: "delivery", content: { query_id: "query-1", references: [reference] } } };
const props = { view: "memory" as const, repository: "Cortex", searchText: "", onSearch: vi.fn(), onNavigate: vi.fn() };
beforeEach(() => { api.mockReset(); api.mockImplementation(async endpoint => { if (endpoint === "browse") return { records: [record], next_cursor: null }; if (endpoint === "get") return record; if (endpoint === "audit/list") return { entries: [{ record: delivery, archived: false }], next_cursor: null }; if (endpoint === "audit/get") return delivery; if (endpoint === "audit/metrics") return {}; return {}; }); });
afterEach(cleanup);
describe("local dashboard views", () => {
  it("uses existing memory cards and renders untrusted body as text", async () => {
    const { container } = render(<LocalWorkspace {...props} />);
    fireEvent.click(await screen.findByText("Preserved memory"));
    await screen.findByText("Source and integrity");
    expect(container.querySelector(".memory-grid .memory")).not.toBeNull();
    expect(container.querySelector("script")).toBeNull();
    expect(screen.getAllByText("<script>unsafe()</script>").length).toBeGreaterThan(0);
  });
  it("captures into exact repository and preserves editor after failure", async () => {
    api.mockImplementation(async endpoint => { if (endpoint === "browse") return { records: [], next_cursor: null }; if (endpoint === "capture") throw new Error("Disk unavailable"); return {}; });
    render(<LocalWorkspace {...props} />);
    fireEvent.click(screen.getByText("New memory"));
    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "New decision" } });
    fireEvent.change(screen.getByLabelText("Body"), { target: { value: "Evidence" } });
    fireEvent.click(screen.getByText("Save memory"));
    await screen.findAllByText("Disk unavailable");
    expect(screen.getByLabelText("Title")).toHaveValue("New decision");
    expect(api).toHaveBeenCalledWith("capture", expect.objectContaining({ repository: "Cortex", title: "New decision", body: "Evidence", kind: "note" }), expect.any(AbortSignal));
  });
  it("retries exact immutable feedback request and ID after transport failure", async () => {
    let attempts = 0;
    const base = api.getMockImplementation()!;
    api.mockImplementation(async (...args) => { if (args[0] === "feedback/record") { if (++attempts === 1) throw new Error("Connection lost"); return { entry: { record: { request: args[1], recorded_at: "now" }, eligibility: "eligible" } }; } return base(...args); });
    render(<LocalWorkspace {...props} view="audit" />);
    fireEvent.click(await screen.findByText("prepared delivery"));
    fireEvent.click(await screen.findByText("Judge exact delivered record"));
    fireEvent.change(screen.getByLabelText("Judgment"), { target: { value: "helpful" } });
    fireEvent.click(screen.getByText("Save feedback"));
    await screen.findByText("Retry exact feedback");
    expect(screen.getByLabelText("Judgment")).toBeDisabled();
    fireEvent.click(screen.getByText("Retry exact feedback"));
    await screen.findByText("Declared feedback saved. This is not proof of model consumption.");
    const calls = api.mock.calls.filter(c => c[0] === "feedback/record");
    expect(calls).toHaveLength(2);
    expect(calls[0][1]).toEqual(calls[1][1]);
    expect(calls[0][1]).toMatchObject({ repository: "Cortex", reference, judgment: "helpful", query_id: "query-1", delivery_id: "delivery-1" });
  });
  it("resets opt-in and detail on scope change; ignores stale response", async () => {
    let resolveOld!: (value: unknown) => void;
    api.mockImplementation(async (endpoint, payload) => {
      if (endpoint === "browse") { if ((payload as { repository: string }).repository === "Cortex") return new Promise(resolve => { resolveOld = resolve; }); return { records: [], next_cursor: null }; }
      return {};
    });
    const { rerender } = render(<LocalWorkspace {...props} />);
    fireEvent.click(screen.getByLabelText("Feedback ranking"));
    rerender(<LocalWorkspace {...props} repository="cortex" />);
    await screen.findByText("No records match this repository and filter.");
    await act(async () => resolveOld({ records: [record], next_cursor: null }));
    expect(screen.queryByText("Preserved memory")).toBeNull();
    expect(screen.getByLabelText("Feedback ranking")).not.toBeChecked();
    expect(api.mock.calls[0][2]?.aborted).toBe(true);
  });
  it("passes opaque cursor and resets pagination when filters change", async () => {
    api.mockResolvedValue({ records: [record], next_cursor: "opaque-page-2" });
    render(<LocalWorkspace {...props} />);
    await screen.findByText("Preserved memory");
    fireEvent.click(screen.getByText("Next"));
    await waitFor(() => expect(api).toHaveBeenCalledWith("browse", expect.objectContaining({ cursor: "opaque-page-2" }), expect.any(AbortSignal)));
    fireEvent.change(screen.getByLabelText("Kind"), { target: { value: "lesson" } });
    await waitFor(() => expect(api).toHaveBeenLastCalledWith("browse", expect.objectContaining({ cursor: null, kind: "lesson" }), expect.any(AbortSignal)));
  });
  it("rejects a mismatched exact historical source", async () => {
    const base = api.getMockImplementation()!;
    api.mockImplementation(async (...args) => args[0] === "get" ? { ...record, content_sha256: "changed" } : base(...args));
    render(<LocalWorkspace {...props} view="audit" />);
    fireEvent.click(await screen.findByText("prepared delivery"));
    fireEvent.click(await screen.findByText("Read exact source revision"));
    await screen.findByText("Historical source does not match the exact audit reference.");
    expect(screen.queryByText("Source and integrity")).toBeNull();
  });
  it("revises with optimistic revision and preserved metadata", async () => {
    const base = api.getMockImplementation()!;
    api.mockImplementation(async (...args) => args[0] === "revise" ? { record: { ...record, revision: 2, title: "Revised" } } : base(...args));
    render(<LocalWorkspace {...props} />);
    fireEvent.click(await screen.findByText("Preserved memory"));
    fireEvent.click(await screen.findByText("Revise memory"));
    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "Revised" } });
    fireEvent.click(screen.getByText("Save memory"));
    await screen.findByText("Memory saved as revision 2.");
    expect(api).toHaveBeenCalledWith("revise", expect.objectContaining({ repository: "Cortex", id: "m1", expected_revision: 1, request: { title: "Revised", body: record.body, authority: "observation", links: [], metadata: {} } }), expect.any(AbortSignal));
  });
  it("queues locally without processing provider work", async () => {
    render(<LocalWorkspace {...props} />);
    fireEvent.click(await screen.findByText("Preserved memory"));
    fireEvent.click(await screen.findByText("Queue classification"));
    await screen.findByText("Classification queued locally. No provider request sent.");
    expect(api).toHaveBeenCalledWith("enqueue", { repository: "Cortex", id: "m1" }, expect.any(AbortSignal));
    expect(api.mock.calls.some(c => c[0].includes("classify"))).toBe(false);
  });
  it("renders explicit graph edges without opening unrelated records", async () => {
    const base = api.getMockImplementation()!;
    api.mockImplementation(async (...args) => args[0] === "graph" ? { records: [record], edges: [], truncated: false } : base(...args));
    render(<LocalWorkspace {...props} view="graph" />);
    fireEvent.click(await screen.findByText("Preserved memory"));
    await screen.findByText("1 records / 0 explicit connections");
    expect(screen.getByLabelText("Explicit memory connections").querySelectorAll("line")).toHaveLength(0);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("clears prior history and graph after capture and resets revision after revise", async () => {
    const previous = { ...record, revision: 4 };
    const saved = { ...record, id: "m2", title: "New memory B", revision: 1 };
    const base = api.getMockImplementation()!;
    api.mockImplementation(async (...args) => {
      if (args[0] === "get") return (args[1] as { revision?: number }).revision ? { ...previous, title: "Historical A only" } : previous;
      if (args[0] === "graph") return { records: [previous], edges: [], truncated: false };
      if (args[0] === "capture") return { record: saved };
      if (args[0] === "revise") return { record: { ...saved, revision: 2 } };
      return base(...args);
    });
    render(<LocalWorkspace {...props} />);
    fireEvent.click(await screen.findByText("Preserved memory"));
    fireEvent.click(await screen.findByText("Read revision"));
    await screen.findByText(/Historical A only/);
    fireEvent.click(screen.getByText("Explore connections"));
    await screen.findByLabelText("Explicit memory connections");
    fireEvent.click(screen.getByLabelText("Close"));
    fireEvent.click(screen.getByText("New memory"));
    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "New memory B" } });
    fireEvent.change(screen.getByLabelText("Body"), { target: { value: "Evidence B" } });
    fireEvent.click(screen.getByText("Save memory"));
    await screen.findByText("Memory saved as revision 1.");
    expect(screen.queryByText(/Historical A only/)).toBeNull();
    expect(screen.queryByLabelText("Explicit memory connections")).toBeNull();
    expect(screen.getByLabelText("Revision")).toHaveValue(1);
    fireEvent.click(screen.getByText("Revise memory"));
    fireEvent.click(screen.getByText("Save memory"));
    await screen.findByText("Memory saved as revision 2.");
    expect(screen.getByLabelText("Revision")).toHaveValue(2);
  });

  it("shows actual scope inventory and compact timestamps with full source time", async () => {
    const base = api.getMockImplementation()!;
    api.mockImplementation(async (...args) => {
      if (args[0] === "status") return { records: 106962, memories: 16, events: 106946, links: 1812, revisions: 106970, repositories: 1 };
      if (args[0] === "browse") return { records: [{ ...record, kind: "event", updated_at: "2026-10-18T11:30:22.123456789Z" }], next_cursor: null };
      return base(...args);
    });
    const { container } = render(<LocalWorkspace {...props} view="timeline" />);
    await screen.findByText("11:30:22");
    const time = container.querySelector("time")!;
    expect(time.title).toBe("2026-10-18T11:30:22.123456789Z");
    expect(time.textContent).toBe("11:30:222026-10-18");
    expect(container.querySelector(".timeline__type svg")).not.toBeNull();
    const stats = container.querySelectorAll(".stats-grid .stat");
    expect(stats).toHaveLength(4);
    expect(Array.from(stats).map(stat => stat.querySelector(".stat__value")?.textContent)).toEqual([106962, 16, 106946, 1812].map(n => n.toLocaleString()));
    expect(api).toHaveBeenCalledWith("status", { repository: "Cortex" }, expect.any(AbortSignal));
    expect(container.querySelector(".stats-grid")!.compareDocumentPosition(container.querySelector(".filter-bar")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("resets pagination before header search changes and preserves kind filter", async () => {
    api.mockImplementation(async endpoint => endpoint === "search" ? [{ record }] : { records: [record], next_cursor: "old-page-2" });
    const { rerender } = render(<LocalWorkspace {...props} />);
    await screen.findByText("Preserved memory");
    fireEvent.change(screen.getByLabelText("Kind"), { target: { value: "note" } });
    await waitFor(() => expect(screen.getByText("Next")).not.toBeDisabled());
    fireEvent.click(screen.getByText("Next"));
    await waitFor(() => expect(api).toHaveBeenLastCalledWith("browse", expect.objectContaining({ cursor: "old-page-2", kind: "note" }), expect.any(AbortSignal)));
    rerender(<LocalWorkspace {...props} searchText="memory" />);
    await screen.findByText("Page 1 / up to 100 search matches");
    expect(screen.getByText("Previous")).toBeDisabled();
    expect(screen.getByLabelText("Kind")).toHaveValue("note");
    api.mockClear();
    rerender(<LocalWorkspace {...props} searchText="" />);
    await waitFor(() => expect(api).toHaveBeenCalledWith("browse", expect.objectContaining({ cursor: null, kind: "note" }), expect.any(AbortSignal)));
    expect(api.mock.calls.filter(c => c[0] === "browse").every(c => (c[1] as { cursor: unknown }).cursor === null)).toBe(true);
    expect(screen.getByText("Page 1")).toBeInTheDocument();
    expect(screen.getByText("Previous")).toBeDisabled();
  });

});
