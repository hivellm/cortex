// Phase11w §4.8 — event-mapper + scope + client unit tests.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { buildFrame, mapEvent } from "../src/events.js";
import { hookUrl, loadConfig } from "../src/config.js";
import { extractBundle, extractDecision, postHook } from "../src/client.js";
import { resolveScope, _resetCache } from "../src/scope.js";

describe("mapEvent", () => {
  it("maps session.created to SessionStart (fire-and-forget)", () => {
    const m = mapEvent("session.created");
    expect(m).toEqual({ kind: "SessionStart", synchronous: false });
  });

  it("maps message.updated to UserPromptSubmit (synchronous)", () => {
    const m = mapEvent("message.updated");
    expect(m).toEqual({ kind: "UserPromptSubmit", synchronous: true });
  });

  it("maps tool.execute.before to PreToolUse (synchronous)", () => {
    const m = mapEvent("tool.execute.before");
    expect(m).toEqual({ kind: "PreToolUse", synchronous: true });
  });

  it("maps session.idle to Stop for parent sessions and SubagentStop for subagents", () => {
    expect(mapEvent("session.idle", false)).toEqual({ kind: "Stop", synchronous: false });
    expect(mapEvent("session.idle", true)).toEqual({ kind: "SubagentStop", synchronous: false });
  });
});

describe("buildFrame", () => {
  it("produces the wire shape the daemon's POST /hook accepts", () => {
    const frame = buildFrame({
      kind: "UserPromptSubmit",
      sessionId: "s1",
      cwd: "/repos/cortex",
      payload: { prompt: "explain" },
    });
    expect(frame).toEqual({
      hook: "UserPromptSubmit",
      session_id: "s1",
      cwd: "/repos/cortex",
      payload: { prompt: "explain" },
    });
  });
});

describe("loadConfig", () => {
  it("defaults to 127.0.0.1:17004 when CORTEX_ADAPTER_HTTP_BIND is unset", () => {
    const cfg = loadConfig({});
    expect(cfg.adapterHttpBind).toBe("127.0.0.1:17004");
    expect(cfg.disabled).toBe(false);
    expect(cfg.preThinkingTimeoutMs).toBe(1500);
    expect(cfg.preThinkingKb).toBe(12);
  });

  it("honours CORTEX_OPENCODE_DISABLE truthy values", () => {
    expect(loadConfig({ CORTEX_OPENCODE_DISABLE: "1" }).disabled).toBe(true);
    expect(loadConfig({ CORTEX_OPENCODE_DISABLE: "true" }).disabled).toBe(true);
    expect(loadConfig({ CORTEX_OPENCODE_DISABLE: "false" }).disabled).toBe(false);
    expect(loadConfig({ CORTEX_OPENCODE_DISABLE: "0" }).disabled).toBe(false);
  });

  it("clamps invalid numeric env to the defaults", () => {
    const cfg = loadConfig({
      CORTEX_OPENCODE_PRE_THINKING_TIMEOUT_MS: "not-a-number",
      CORTEX_OPENCODE_PRE_THINKING_KB: "-5",
    });
    expect(cfg.preThinkingTimeoutMs).toBe(1500);
    expect(cfg.preThinkingKb).toBe(12);
  });
});

describe("hookUrl", () => {
  it("prepends http:// when the bind has no scheme", () => {
    const cfg = loadConfig({});
    expect(hookUrl(cfg)).toBe("http://127.0.0.1:17004/hook");
  });

  it("respects an explicit scheme", () => {
    const cfg = loadConfig({ CORTEX_ADAPTER_HTTP_BIND: "http://localhost:9000" });
    expect(hookUrl(cfg)).toBe("http://localhost:9000/hook");
  });
});

describe("postHook", () => {
  it("returns the daemon's HookResponse on a 2xx body", async () => {
    const cfg = loadConfig({});
    const fetchFn = vi.fn(async () => ({
      ok: true,
      json: async () => ({
        hookSpecificOutput: { additionalContext: "## active work\n- t1\n" },
      }),
    })) as unknown as typeof fetch;
    const resp = await postHook({ hook: "UserPromptSubmit" }, cfg, fetchFn);
    expect(extractBundle(resp)).toContain("active work");
  });

  it("fails open with an empty response on network error", async () => {
    const cfg = loadConfig({});
    const fetchFn = vi.fn(async () => {
      throw new Error("ECONNREFUSED");
    }) as unknown as typeof fetch;
    const resp = await postHook({ hook: "UserPromptSubmit" }, cfg, fetchFn);
    expect(extractBundle(resp)).toBe("");
    expect(extractDecision(resp).decision).toBe("ask");
  });

  it("fails open with an empty response on non-2xx", async () => {
    const cfg = loadConfig({});
    const fetchFn = vi.fn(async () => ({
      ok: false,
      json: async () => ({ should: "not be read" }),
    })) as unknown as typeof fetch;
    const resp = await postHook({ hook: "UserPromptSubmit" }, cfg, fetchFn);
    expect(extractBundle(resp)).toBe("");
  });

  it("short-circuits when the kill-switch is set", async () => {
    const cfg = loadConfig({ CORTEX_OPENCODE_DISABLE: "1" });
    const fetchFn = vi.fn() as unknown as typeof fetch;
    const resp = await postHook({ hook: "UserPromptSubmit" }, cfg, fetchFn);
    expect(extractBundle(resp)).toBe("");
    expect((fetchFn as unknown as { mock: { calls: unknown[] } }).mock.calls.length).toBe(0);
  });
});

describe("resolveScope", () => {
  beforeEach(() => _resetCache());

  it("derives repo from the directory basename when .git is absent", async () => {
    const fakeFs = {
      readFile: async () => {
        throw new Error("ENOENT");
      },
    };
    const scope = await resolveScope("s1", { directory: "/repos/cortex" }, fakeFs);
    expect(scope.repo).toBe("cortex");
    expect(scope.branch).toBeUndefined();
  });

  it("reads the active branch from .git/HEAD", async () => {
    const fakeFs = {
      readFile: async (p: string) => {
        expect(p).toBe("/repos/cortex/.git/HEAD");
        return "ref: refs/heads/main\n";
      },
    };
    const scope = await resolveScope("s2", { directory: "/repos/cortex" }, fakeFs);
    expect(scope).toEqual({ repo: "cortex", branch: "main" });
  });

  it("surfaces a detached HEAD as the short sha", async () => {
    const fakeFs = {
      readFile: async () => "deadbeefcafebabe1234567890abcdef\n",
    };
    const scope = await resolveScope("s3", { directory: "/x" }, fakeFs);
    expect(scope.branch).toBe("deadbee");
  });

  it("caches per session id", async () => {
    let calls = 0;
    const fakeFs = {
      readFile: async () => {
        calls += 1;
        return "ref: refs/heads/feature-x\n";
      },
    };
    await resolveScope("s4", { directory: "/y" }, fakeFs);
    await resolveScope("s4", { directory: "/y" }, fakeFs);
    expect(calls).toBe(1);
  });
});
