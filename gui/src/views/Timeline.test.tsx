// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";

// Phase3 — vitest covers the Inspector contract:
// - `Content` section renders only for `kind === "tool_call"`
// - `content_hash` row + filter button surfaces the hash and toggles
//   the active filter
// - the truncated marker links to the per-id full-body route

vi.mock("../atoms/Icon", () => ({
  // The real Icon imports an SVG sprite; the test rig stubs it so
  // the Inspector renders without bundling Vite asset URLs.
  Icon: ({ name }: { name: string }) => <span data-testid={`icon-${name}`} />,
}));

import { Inspector } from "./Timeline";
import type { TimelineEvent } from "../lib/api";

afterEach(() => {
  cleanup();
});

function toolCallEvent(overrides: Partial<TimelineEvent> = {}): TimelineEvent {
  return {
    id: "archive|01HX",
    t: "12:00:00",
    kind: "tool_call",
    title: "[Edit]",
    detail: "[Edit] short",
    repo: "Cortex",
    session_id: "01HSESSION",
    model: "claude-code",
    content_hash: "sha256:abc1234567890def",
    preview: "tool body that the inspector should surface",
    preview_truncated: false,
    ...overrides,
  };
}

describe("Inspector — phase3 hash + preview", () => {
  it("renders the Content section only for tool_call rows", () => {
    const ev = toolCallEvent();
    render(
      <Inspector ev={ev} onClose={() => {}} onFilterByHash={() => {}} />,
    );
    expect(screen.getByTestId("inspector-content")).toBeInTheDocument();
    expect(screen.getByTestId("inspector-content-pre").textContent).toBe(
      ev.preview,
    );
  });

  it("omits the Content section for non-tool_call rows", () => {
    const turn: TimelineEvent = {
      ...toolCallEvent(),
      kind: "turn",
      title: "user prompt",
      detail: "the prompt body",
      // Phase3 contract: server omits `preview` for non-tool_call rows.
      preview: undefined,
      preview_truncated: false,
      content_hash: undefined,
    };
    render(
      <Inspector ev={turn} onClose={() => {}} onFilterByHash={() => {}} />,
    );
    expect(screen.queryByTestId("inspector-content")).toBeNull();
  });

  it("surfaces the truncated marker with a link to the full body", () => {
    const ev = toolCallEvent({
      preview: "head of clipped body",
      preview_truncated: true,
    });
    render(
      <Inspector ev={ev} onClose={() => {}} onFilterByHash={() => {}} />,
    );
    const link = screen.getByText("open full");
    expect(link).toBeInTheDocument();
    expect(link.getAttribute("href")).toBe(
      `/v1/dashboard/timeline/${encodeURIComponent(ev.id)}`,
    );
  });

  it("renders the content_hash row with the short form and triggers the filter on click", () => {
    const ev = toolCallEvent();
    const onFilter = vi.fn();
    render(
      <Inspector ev={ev} onClose={() => {}} onFilterByHash={onFilter} />,
    );
    const row = screen.getByTestId("inspector-hash-row");
    // Short form keeps the algo prefix and the first 7 hex chars.
    expect(row.textContent).toContain("sha256:abc1234…");
    const button = row.querySelector("button.link-btn") as HTMLButtonElement;
    fireEvent.click(button);
    expect(onFilter).toHaveBeenCalledWith(ev.content_hash);
  });

  it("omits the hash row when the envelope dropped content_hash", () => {
    const ev = toolCallEvent({ content_hash: undefined });
    render(
      <Inspector ev={ev} onClose={() => {}} onFilterByHash={() => {}} />,
    );
    expect(screen.queryByTestId("inspector-hash-row")).toBeNull();
  });
});
