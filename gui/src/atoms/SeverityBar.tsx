/**
 * Three-segment severity indicator. Ported from
 * `gui/assets/atoms.jsx`. `info` lights 1, `notable` lights 2,
 * `critical` lights 3 — both the segment count and the color follow
 * the severity level so the bar reads at a glance even without the
 * adjacent label.
 */

export type Severity = "info" | "notable" | "warn" | "critical";

const SEG_COUNT: Record<string, number> = {
  info: 1,
  notable: 2,
  warn: 2,
  critical: 3,
};

const COLOR: Record<string, string> = {
  info: "var(--info)",
  notable: "var(--warn)",
  warn: "var(--warn)",
  critical: "var(--critical)",
};

export function SeverityBar({ severity }: { severity: string }) {
  const count = SEG_COUNT[severity] ?? 1;
  const color = COLOR[severity] ?? "var(--info)";
  return (
    <span className="severity-bar" style={{ color }}>
      {[0, 1, 2].map((i) => (
        <span key={i} className={`seg ${i < count ? "is-on" : ""}`} />
      ))}
    </span>
  );
}
