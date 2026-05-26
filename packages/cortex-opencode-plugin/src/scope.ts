// Phase11w §4.4 — repo / branch derivation cached per session.
//
// Mirrors the Rust adapter's `dispatcher::scope` heuristics: read
// `<cwd>/.git/HEAD`, the working-tree dir basename, and the
// `worktree` hint when present. Cached per `session_id` so the
// plugin only walks the filesystem once per session.

export interface ScopeHint {
  /** Absolute working directory. */
  directory: string;
  /** OpenCode `worktree` hint when present (a relative path inside `directory`). */
  worktree?: string;
}

export interface Scope {
  /** Repo slug derived from the directory basename or git remote. */
  repo: string;
  /** Active branch from `<cwd>/.git/HEAD`, when resolvable. */
  branch?: string;
}

const cache = new Map<string, Scope>();

/**
 * Resolve scope for `sessionId` + `hint`. Results are cached per
 * session id so repeated calls (one per envelope) are O(1).
 *
 * Branch resolution is best-effort — on any error the plugin
 * returns `{repo}` without a branch and the daemon's scope derivation
 * fills in whatever it can on its side.
 */
export async function resolveScope(
  sessionId: string,
  hint: ScopeHint,
  fs: { readFile: (p: string) => Promise<string> } = {
    readFile: async (p) => (await import("node:fs/promises")).readFile(p, "utf8"),
  },
): Promise<Scope> {
  const cached = cache.get(sessionId);
  if (cached) return cached;

  const dir = hint.worktree ? `${hint.directory.replace(/[/\\]+$/, "")}/${hint.worktree}` : hint.directory;
  const basename = dir.split(/[\\/]/).filter(Boolean).pop() ?? dir;
  let branch: string | undefined;
  try {
    const head = await fs.readFile(`${dir}/.git/HEAD`);
    const trimmed = head.trim();
    if (trimmed.startsWith("ref:")) {
      // `ref: refs/heads/<branch>` → `<branch>`
      const parts = trimmed.slice(4).trim().split("/");
      branch = parts.slice(2).join("/") || undefined;
    } else if (/^[0-9a-f]{7,40}$/i.test(trimmed)) {
      // Detached HEAD — surface the short sha as the branch label.
      branch = trimmed.slice(0, 7);
    }
  } catch {
    // No .git or unreadable HEAD — keep `branch` undefined.
  }

  const scope: Scope = branch ? { repo: basename, branch } : { repo: basename };
  cache.set(sessionId, scope);
  return scope;
}

/** Drop the cache entry for `sessionId`. Used on session.idle (Stop). */
export function forgetScope(sessionId: string): void {
  cache.delete(sessionId);
}

/** Test-only — reset the entire cache. */
export function _resetCache(): void {
  cache.clear();
}
