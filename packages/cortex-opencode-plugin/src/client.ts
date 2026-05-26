// Phase11w §4.3 — daemon HTTP client.
//
// `postHook` sends one `HookFrame` to the daemon's HTTP listener,
// awaits the response within `cfg.preThinkingTimeoutMs`, and returns
// the parsed `HookResponse`. On any network error / timeout / non-2xx
// the plugin returns a fail-open empty response so the session never
// breaks (parity with the Claude Code adapter's hook contract).

import { hookUrl, type CortexOpenCodeConfig } from "./config.js";

/** Subset of the daemon's `HookResponse` the plugin cares about. */
export interface HookResponse {
  hookSpecificOutput?: {
    hookEventName?: string;
    additionalContext?: string;
  };
  permissionDecision?: "allow" | "ask" | "deny";
  permissionDecisionReason?: string;
}

const EMPTY: HookResponse = Object.freeze({}) as HookResponse;

/**
 * POST `frame` to the daemon. `synchronous` decides whether the
 * caller needs the response (UserPromptSubmit / PreToolUse) or just
 * fire-and-forget (PostToolUse / SubagentStop / Stop). Either way
 * the function never throws — a failure surfaces as a fail-open
 * empty response.
 */
export async function postHook(
  frame: Record<string, unknown>,
  cfg: CortexOpenCodeConfig,
  fetchFn: typeof fetch = fetch,
): Promise<HookResponse> {
  if (cfg.disabled) {
    return EMPTY;
  }
  const url = hookUrl(cfg);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), cfg.preThinkingTimeoutMs);
  try {
    const resp = await fetchFn(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(frame),
      signal: controller.signal,
    });
    if (!resp.ok) {
      return EMPTY;
    }
    const body = (await resp.json()) as HookResponse;
    return body ?? EMPTY;
  } catch {
    // Network error / timeout / abort — fail open.
    return EMPTY;
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Extract the Markdown bundle from a `HookResponse`. Returns an
 * empty string when the response carries no bundle so the caller
 * can short-circuit the TUI append.
 */
export function extractBundle(resp: HookResponse): string {
  return resp.hookSpecificOutput?.additionalContext ?? "";
}

/**
 * Extract the permission verdict from a `HookResponse`. Defaults
 * to `"ask"` (the fail-safe path) when the daemon did not surface
 * one, matching the Claude Code adapter's contract.
 */
export function extractDecision(
  resp: HookResponse,
): { decision: "allow" | "ask" | "deny"; reason?: string } {
  return {
    decision: resp.permissionDecision ?? "ask",
    reason: resp.permissionDecisionReason,
  };
}
