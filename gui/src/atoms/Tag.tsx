import type { ReactNode } from "react";

export type TagTone = "default" | "ok" | "warn" | "critical" | "info" | "accent";

type TagProps = { children: ReactNode; tone?: TagTone };

export function Tag({ children, tone = "default" }: TagProps) {
  return (
    <span className={`tag ${tone !== "default" ? `tag--${tone}` : ""}`}>{children}</span>
  );
}
