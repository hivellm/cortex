type SparklineProps = {
  data: number[];
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
  const max = Math.max(...data);
  const min = Math.min(...data);
  const range = max - min || 1;
  const step = (w - pad * 2) / (data.length - 1 || 1);
  const pts = data.map((v, i): [number, number] => [
    pad + i * step,
    h - pad - ((v - min) / range) * (h - pad * 2),
  ]);
  const path = pts
    .map(([x, y], i) => (i === 0 ? `M${x},${y}` : `L${x},${y}`))
    .join(" ");
  const last = pts[pts.length - 1];
  const first = pts[0];
  const fill = `${path} L${last[0]},${h} L${first[0]},${h} Z`;
  return (
    <svg className="spark" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      {filled ? <path d={fill} fill={color} opacity="0.15" /> : null}
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
