/** Number formatter — `1234` → `"1.2k"`, `1_200_000` → `"1.2M"`. */
export function fmtNum(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

/** Severity → CSS color token used by the prototype's design system. */
export function sevTone(severity: string | null | undefined): string {
  switch (severity) {
    case "critical":
      return "var(--critical)";
    case "notable":
      return "var(--warn)";
    case "info":
      return "var(--info)";
    default:
      return "var(--fg-3)";
  }
}

/** Kind → human label for UI surfaces. */
export function kindLabel(kind: string): string {
  switch (kind) {
    case "turn":
      return "Turn";
    case "tool_call":
      return "Tool";
    case "agent_call":
      return "Agent";
    case "memory":
      return "Memory";
    case "decision":
      return "Decision";
    case "law_violation":
      return "Law violation";
    case "analysis":
      return "Analysis";
    case "artifact":
      return "Artifact";
    default:
      return kind;
  }
}
