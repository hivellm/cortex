// Phase11w §4.6 — `CortexPlugin` async factory.
//
// Subscribes to the OpenCode lifecycle events documented in spec 20
// and posts envelopes to the daemon's HTTP listener. Pre-thinking
// injection uses `tui.prompt.append` per the spike answer 1.6 (Path A).
//
// The plugin is fail-open at every site: a daemon timeout, a network
// error, or a missing TUI API surfaces as a WARN log and the session
// continues. The kill-switch `CORTEX_OPENCODE_DISABLE=1` short-circuits
// every handler.

import {
  postHook,
  extractBundle,
  extractDecision,
  type HookResponse,
} from "./client.js";
import { loadConfig, type CortexOpenCodeConfig } from "./config.js";
import { buildFrame, mapEvent, type HookKind } from "./events.js";
import { forgetScope, resolveScope } from "./scope.js";

/**
 * Subset of the OpenCode plugin context the plugin reads. The actual
 * shape is supplied by `@opencode-ai/plugin`; we type only what we
 * use so the package builds without pulling the peer dep in as a
 * hard dependency.
 */
export interface OpenCodeContext {
  project?: { id?: string };
  directory: string;
  worktree?: string;
  client?: {
    tui?: { prompt: { append(text: string): Promise<void> } };
  };
  on<K extends string>(event: K, handler: (payload: any) => any): void;
}

/** Plugin returned by `default export`. */
export type CortexPluginFactory = (ctx: OpenCodeContext) => Promise<void>;

const seenUserMessages = new Set<string>();

/**
 * Build a fully-initialised plugin. The factory subscribes to every
 * lifecycle event it cares about + caches the resolved config so
 * handlers do not re-walk env.
 */
export const CortexPlugin: CortexPluginFactory = async (ctx) => {
  const cfg = loadConfig();
  if (cfg.disabled) {
    // eslint-disable-next-line no-console
    console.warn("[cortex-opencode] CORTEX_OPENCODE_DISABLE=1; plugin inactive");
    return;
  }

  // session.created → SessionStart (fire-and-forget).
  ctx.on("session.created", async (payload: { session: { id: string } }) => {
    await sendFrame(cfg, "SessionStart", payload.session.id, ctx, payload);
  });

  // message.updated → UserPromptSubmit when the message is the user's
  // first text part (per the spike answer a). The plugin also reads
  // the daemon's response and appends the bundle through
  // `tui.prompt.append` (per the spike answer c + decision 1.6 Path A).
  ctx.on(
    "message.updated",
    async (payload: {
      message: {
        id: string;
        role: "user" | "assistant" | "system";
        session_id: string;
        parts?: Array<{ type: string; text?: string }>;
      };
    }) => {
      const m = payload.message;
      if (m.role !== "user") return;
      if (!m.parts || !m.parts.some((p) => p.type === "text" && (p.text ?? "").length > 0)) return;
      if (seenUserMessages.has(m.id)) return;
      seenUserMessages.add(m.id);

      const resp = await sendFrame(cfg, "UserPromptSubmit", m.session_id, ctx, {
        prompt: m.parts.find((p) => p.type === "text")?.text ?? "",
        message_id: m.id,
      });
      const bundle = extractBundle(resp);
      if (bundle && ctx.client?.tui?.prompt?.append) {
        try {
          await ctx.client.tui.prompt.append(bundle);
        } catch (e) {
          // eslint-disable-next-line no-console
          console.warn("[cortex-opencode] tui.prompt.append failed:", e);
        }
      }
    },
  );

  // tool.execute.before → PreToolUse (synchronous; deny short-circuits).
  ctx.on(
    "tool.execute.before",
    async (payload: { session: { id: string }; tool: { name: string; input?: unknown } }) => {
      const resp = await sendFrame(cfg, "PreToolUse", payload.session.id, ctx, {
        tool_name: payload.tool.name,
        input: payload.tool.input ?? {},
      });
      const { decision, reason } = extractDecision(resp);
      if (decision === "deny") {
        throw new Error(`[cortex law-check] ${reason ?? "tool denied by Cortex law-check"}`);
      }
    },
  );

  // tool.execute.after → PostToolUse (fire-and-forget).
  ctx.on(
    "tool.execute.after",
    async (payload: {
      session: { id: string };
      tool: { name: string };
      result?: unknown;
      error?: string;
    }) => {
      await sendFrame(cfg, "PostToolUse", payload.session.id, ctx, {
        tool_name: payload.tool.name,
        result: payload.result ?? null,
        error: payload.error ?? null,
      });
    },
  );

  // permission.asked → law-check round-trip (synchronous; verdict drives
  // the OpenCode runtime's allow/ask/deny). Defaults to "ask" on any
  // daemon failure so the session never breaks.
  ctx.on(
    "permission.asked",
    async (payload: {
      session: { id: string };
      tool: { name: string; input?: unknown };
      reply: (decision: "allow" | "ask" | "deny", reason?: string) => Promise<void>;
    }) => {
      const resp = await sendFrame(cfg, "PreToolUse", payload.session.id, ctx, {
        tool_name: payload.tool.name,
        input: payload.tool.input ?? {},
      });
      const { decision, reason } = extractDecision(resp);
      await payload.reply(decision, reason);
    },
  );

  // session.idle → Stop / SubagentStop (per spike answer e).
  ctx.on(
    "session.idle",
    async (payload: { session: { id: string; parent_id?: string } }) => {
      const isSubagent = !!payload.session.parent_id;
      const mapped = mapEvent("session.idle", isSubagent);
      if (!mapped) return;
      await sendFrame(cfg, mapped.kind, payload.session.id, ctx, {
        parent_session_id: payload.session.parent_id ?? null,
      });
      if (!isSubagent) {
        forgetScope(payload.session.id);
      }
    },
  );
};

async function sendFrame(
  cfg: CortexOpenCodeConfig,
  kind: HookKind,
  sessionId: string,
  ctx: OpenCodeContext,
  payload: Record<string, unknown>,
): Promise<HookResponse> {
  const scope = await resolveScope(sessionId, { directory: ctx.directory, worktree: ctx.worktree });
  const enriched = { ...payload, repo: scope.repo, branch: scope.branch ?? null };
  const frame = buildFrame({
    kind,
    sessionId,
    cwd: ctx.worktree ? `${ctx.directory}/${ctx.worktree}` : ctx.directory,
    payload: enriched,
  });
  return postHook(frame, cfg);
}

export default CortexPlugin;
