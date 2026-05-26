// Phase11w §4.5 — env-knob loading.
//
// Mirrors the env contract documented in spec 20 § Configuration.
// `loadConfig()` is pure (reads `process.env` once, returns a frozen
// snapshot) so the rest of the plugin never re-reads the environment
// per call.

export interface CortexOpenCodeConfig {
  /** Daemon HTTP listener bind address. */
  adapterHttpBind: string;
  /** When true the plugin is a no-op (kill-switch parity with Claude Code). */
  disabled: boolean;
  /** Wall budget for the pre-thinking round-trip, in milliseconds. */
  preThinkingTimeoutMs: number;
  /** Soft cap on the bundle bytes the plugin appends to the TUI. */
  preThinkingKb: number;
}

const DEFAULT_HTTP_BIND = "127.0.0.1:17004";
const DEFAULT_TIMEOUT_MS = 1500;
const DEFAULT_KB = 12;

function readBool(env: string | undefined): boolean {
  if (env == null) return false;
  const norm = env.trim().toLowerCase();
  return norm === "1" || norm === "true" || norm === "yes" || norm === "on";
}

function readU32(env: string | undefined, fallback: number): number {
  if (env == null) return fallback;
  const n = Number.parseInt(env, 10);
  if (!Number.isFinite(n) || n < 0) return fallback;
  return n;
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): CortexOpenCodeConfig {
  return Object.freeze({
    adapterHttpBind: env.CORTEX_ADAPTER_HTTP_BIND?.trim() || DEFAULT_HTTP_BIND,
    disabled: readBool(env.CORTEX_OPENCODE_DISABLE),
    preThinkingTimeoutMs: readU32(env.CORTEX_OPENCODE_PRE_THINKING_TIMEOUT_MS, DEFAULT_TIMEOUT_MS),
    preThinkingKb: readU32(env.CORTEX_OPENCODE_PRE_THINKING_KB, DEFAULT_KB),
  });
}

/** Resolve the daemon hook endpoint URL from the config. */
export function hookUrl(cfg: CortexOpenCodeConfig): string {
  const bind = cfg.adapterHttpBind.startsWith("http://") || cfg.adapterHttpBind.startsWith("https://")
    ? cfg.adapterHttpBind
    : `http://${cfg.adapterHttpBind}`;
  return `${bind.replace(/\/$/, "")}/hook`;
}
