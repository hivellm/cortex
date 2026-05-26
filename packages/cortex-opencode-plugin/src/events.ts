// Phase11w §4.2 — OpenCode event name → adapter `HookKind` mapping.
//
// `HookKind` strings here MUST match the Rust enum names verbatim
// (`crates/cortex-adapter-claude-code/src/events.rs::HookKind`,
// PascalCase via `#[serde(rename_all = "PascalCase")]`). The daemon
// only branches on the wire string; a typo here lands as a silent
// no-op.

/** Adapter `HookKind` discriminator. */
export type HookKind =
  | "SessionStart"
  | "UserPromptSubmit"
  | "PreToolUse"
  | "PostToolUse"
  | "SubagentStop"
  | "Stop"
  | "Notification";

/** Subset of OpenCode lifecycle events the plugin subscribes to. */
export type OpenCodeEvent =
  | "session.created"
  | "message.updated"
  | "tool.execute.before"
  | "tool.execute.after"
  | "permission.asked"
  | "session.idle"
  | "notification";

export interface MappedFrame {
  kind: HookKind;
  /** When true the caller must await the daemon round-trip (e.g. PreToolUse / UserPromptSubmit). */
  synchronous: boolean;
}

/**
 * Phase11w §4.2 — map an OpenCode event to an adapter `HookKind`.
 *
 * Some OpenCode events do not have a 1:1 mapping (e.g.
 * `message.updated` fires for BOTH user prompts and assistant
 * replies). The caller is responsible for pre-filtering before
 * calling this mapper; the mapper only knows the wire-name
 * relationship.
 */
export function mapEvent(event: OpenCodeEvent, isSubagent = false): MappedFrame | null {
  switch (event) {
    case "session.created":
      return { kind: "SessionStart", synchronous: false };
    case "message.updated":
      return { kind: "UserPromptSubmit", synchronous: true };
    case "tool.execute.before":
      return { kind: "PreToolUse", synchronous: true };
    case "tool.execute.after":
      return { kind: "PostToolUse", synchronous: false };
    case "permission.asked":
      return { kind: "PreToolUse", synchronous: true };
    case "session.idle":
      return { kind: isSubagent ? "SubagentStop" : "Stop", synchronous: false };
    case "notification":
      return { kind: "Notification", synchronous: false };
    default:
      return null;
  }
}

/** Build the `HookFrame` JSON the daemon expects on `POST /hook`. */
export function buildFrame(args: {
  kind: HookKind;
  sessionId: string;
  cwd: string;
  payload: Record<string, unknown>;
}): Record<string, unknown> {
  return {
    hook: args.kind,
    session_id: args.sessionId,
    cwd: args.cwd,
    payload: args.payload,
  };
}
