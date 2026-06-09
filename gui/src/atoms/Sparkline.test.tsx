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

  it("bridgeGaps spreads present samples across the full width (fills the card)", () => {
    // Leading null + two present samples. The present points must span
    // the full width — pad(1) .. w-pad(119) — so a sparse, recent-only
    // series fills the card instead of hugging the right edge.
    const { container } = render(
      <Sparkline data={[null, 10, 20]} filled={false} bridgeGaps />,
    );
    const d = linePath(container);
    expect(d).toContain("M1,"); // first present sample at the left edge
    expect(d).toContain("L119,"); // last present sample at the right edge
  });

  it("bridgeGaps fills the width even when all data is in the trailing buckets", () => {
    // 5 buckets, only the last two present — must still reach x=1.
    const { container } = render(
      <Sparkline data={[null, null, null, 5, 9]} filled={false} bridgeGaps />,
    );
    expect(linePath(container)).toContain("M1,");
  });

  it("still renders nothing when every bucket is null", () => {
    const { container } = render(
      <Sparkline data={[null, null]} bridgeGaps />,
    );
    expect(container.querySelector("svg")).toBeNull();
  });
});
