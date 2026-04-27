/**
 * Mini vertical bar chart. Ported from `gui/assets/atoms.jsx`. Each
 * bar's height is a percentage (0–100), so the caller normalises the
 * series before passing it in.
 */

type BarsProps = {
  data: number[];
  color?: string;
};

export function Bars({ data, color = "var(--accent-dim)" }: BarsProps) {
  return (
    <div className="bars">
      {data.map((v, i) => (
        <div
          key={i}
          className="bars__bar"
          style={{
            height: `${Math.max(0, Math.min(100, v))}%`,
            background: color,
          }}
        />
      ))}
    </div>
  );
}
