type SparklineProps = {
  /** Accepts `null` slots so empty buckets render as a gap instead
   * of a dishonest zero — used by `pre_thinking_p95_ms`. */
  data: (number | null)[];
  color?: string;
  filled?: boolean;
  height?: number;
  /** When `true`, drop `null` buckets and spread the present samples
   * evenly across the full width — one continuous line. Right for a
   * gauge (e.g. P95 latency) where a sample-less minute means "no new
   * reading", not "value dropped". A series with only recent readings
   * then fills the card like the dense zero-filled count series
   * instead of hugging the right edge. Trades exact time-axis spacing
   * (the sparkline has no axis labels) for a gap-free trend; it never
   * fabricates a zero floor. */
  bridgeGaps?: boolean;
};

export function Sparkline({
  data,
  color = "var(--accent)",
  filled = true,
  height = 28,
  bridgeGaps = false,
}: SparklineProps) {
  if (!data || data.length === 0) return null;
  const w = 120;
  const h = height;
  const pad = 1;
  const present = data.filter((v): v is number => v != null);
  if (present.length === 0) return null;
  const max = Math.max(...present);
  const min = Math.min(...present);
  const range = max - min || 1;
  const yOf = (v: number) => h - pad - ((v - min) / range) * (h - pad * 2);
  const segments: [number, number][][] = [];
  if (bridgeGaps) {
    // Gauge mode: drop the null buckets entirely and spread the
    // present samples evenly across the FULL width, so a series that
    // only has recent readings still fills the card like the dense
    // zero-filled count series instead of hugging the right edge. The
    // sparkline is a trend glyph (no time-axis labels), so compressing
    // the sample-less span is the honest trade — it shows the shape of
    // the readings that exist without fabricating a floor.
    const pstep = (w - pad * 2) / (present.length - 1 || 1);
    segments.push(present.map((v, j) => [pad + j * pstep, yOf(v)]));
  } else {
    // Honest time axis: x = bucket index; "lift the pen" over null
    // buckets so a missing slot reads as a gap, not a fake floor.
    const step = (w - pad * 2) / (data.length - 1 || 1);
    let current: [number, number][] = [];
    data.forEach((v, i) => {
      if (v == null) {
        if (current.length > 0) {
          segments.push(current);
          current = [];
        }
        return;
      }
      current.push([pad + i * step, yOf(v)]);
    });
    if (current.length > 0) segments.push(current);
  }
  if (segments.length === 0) return null;
  const path = segments
    .map((seg) =>
      seg
        .map(([x, y], i) => (i === 0 ? `M${x},${y}` : `L${x},${y}`))
        .join(" "),
    )
    .join(" ");
  // Build the fill polygon from the contiguous segment that actually
  // touches the right edge — keeps the shaded area honest when the
  // series has gaps.
  let fillPath: string | null = null;
  if (filled) {
    const fillPaths = segments.map((seg) => {
      const last = seg[seg.length - 1];
      const first = seg[0];
      const lineSeg = seg
        .map(([x, y], i) => (i === 0 ? `M${x},${y}` : `L${x},${y}`))
        .join(" ");
      return `${lineSeg} L${last[0]},${h} L${first[0]},${h} Z`;
    });
    fillPath = fillPaths.join(" ");
  }
  return (
    <svg className="spark" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      {fillPath ? <path d={fillPath} fill={color} opacity="0.15" /> : null}
      <path
        d={path}
        fill="none"
        stroke={color}
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
