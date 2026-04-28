type SparklineProps = {
  /** Accepts `null` slots so empty buckets render as a gap instead
   * of a dishonest zero — used by `pre_thinking_p95_ms`. */
  data: (number | null)[];
  color?: string;
  filled?: boolean;
  height?: number;
};

export function Sparkline({
  data,
  color = "var(--accent)",
  filled = true,
  height = 28,
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
  const step = (w - pad * 2) / (data.length - 1 || 1);
  // Walk the series and emit `M<x>,<y>` whenever we cross a null gap
  // — that produces a polyline that "lifts the pen" over missing
  // buckets instead of drawing a fake floor.
  const segments: [number, number][][] = [];
  let current: [number, number][] = [];
  data.forEach((v, i) => {
    if (v == null) {
      if (current.length > 0) {
        segments.push(current);
        current = [];
      }
      return;
    }
    current.push([
      pad + i * step,
      h - pad - ((v - min) / range) * (h - pad * 2),
    ]);
  });
  if (current.length > 0) segments.push(current);
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
