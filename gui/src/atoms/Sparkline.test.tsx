// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { Sparkline } from "./Sparkline";

afterEach(cleanup);

// The stroke (line) path is the one with fill="none"; the optional
// fill polygon uses fill={color}.
function linePath(container: HTMLElement): string {
  const p = container.querySelector('path[fill="none"]');
  return p?.getAttribute("d") ?? "";
}

// Each `M` command starts a fresh sub-path: one `M` == one continuous
// segment, so counting them tells us whether a gap lifted the pen.
function moveCount(d: string): number {
  return (d.match(/M/g) ?? []).length;
}

describe("Sparkline gap handling", () => {
  it("lifts the pen over null buckets by default (one segment per run)", () => {
    const { container } = render(
      <Sparkline data={[10, null, 20]} filled={false} />,
    );
    expect(moveCount(linePath(container))).toBe(2);
  });

  it("bridges across null buckets into one continuous line when bridgeGaps is set", () => {
    const { container } = render(
      <Sparkline data={[10, null, 20]} filled={false} bridgeGaps />,
    );
    expect(moveCount(linePath(container))).toBe(1);
  });

  it("bridgeGaps preserves the original bucket x-positions (gap spans real time)", () => {
    // 3 buckets over width 120 (pad 1): step = 118/2 = 59.
    // bucket 0 -> x=1, bucket 2 -> x=119. The bridged line must span
    // both ends, not collapse the two points to adjacent x's.
    const { container } = render(
      <Sparkline data={[10, null, 20]} filled={false} bridgeGaps />,
    );
    const d = linePath(container);
    expect(d).toContain("M1,");
    expect(d).toContain("L119,");
  });

  it("still renders nothing when every bucket is null", () => {
    const { container } = render(
      <Sparkline data={[null, null]} bridgeGaps />,
    );
    expect(container.querySelector("svg")).toBeNull();
  });
});
