const TOKEN_KEY = "cortex.local.token";
const REPOSITORY_KEY = "cortex.local.repository";
const VALID_TOKEN = /^[a-fA-F0-9]{64}$/;
let token = "";
export let initialRepository = "";

export function watchLocalStartupNavigation(reload: () => void = () => window.location.reload()): () => void {
  const onHashChange = () => {
    if (document.querySelector<HTMLMetaElement>('meta[name="cortex-runtime"]')?.content !== "standalone") return;
    const nextToken = new URLSearchParams(window.location.hash.slice(1)).get("token");
    // Preserve the fragment until document bootstrap, including when storage is blocked.
    if (nextToken && VALID_TOKEN.test(nextToken)) reload();
  };
  window.addEventListener("hashchange", onHashChange);
  return () => window.removeEventListener("hashchange", onHashChange);
}

export function bootstrapLocalSession(): void {
  if (document.querySelector<HTMLMetaElement>('meta[name="cortex-runtime"]')?.content !== "standalone") return;
  const fragment = new URLSearchParams(window.location.hash.slice(1));
  const suppliedToken = fragment.get("token");
  const suppliedRepository = fragment.get("repository");
  // Remove credentials before mounting React or starting any requests.
  if (window.location.hash) history.replaceState(null, "", location.pathname + location.search);
  try {
    if (suppliedToken !== null) {
      sessionStorage.removeItem(TOKEN_KEY);
      sessionStorage.removeItem(REPOSITORY_KEY);
    }
    token = suppliedToken ?? sessionStorage.getItem(TOKEN_KEY) ?? "";
    initialRepository = suppliedRepository ?? (suppliedToken === null ? sessionStorage.getItem(REPOSITORY_KEY) : null) ?? "";
    if (!VALID_TOKEN.test(token)) token = "";
    if (token) sessionStorage.setItem(TOKEN_KEY, token);
    sessionStorage.setItem(REPOSITORY_KEY, initialRepository);
  } catch {
    token = suppliedToken && VALID_TOKEN.test(suppliedToken) ? suppliedToken : "";
    initialRepository = suppliedRepository ?? "";
  }
}

export async function localRequest<T>(endpoint: string, payload: Record<string, unknown>, signal?: AbortSignal): Promise<T> {
  if (!/^[a-z][a-z0-9_-]*(?:\/[a-z][a-z0-9_-]*)*$/.test(endpoint)) throw new Error("Invalid local endpoint");
  if (!token) throw new Error("Open the startup URL printed by cortex serve to authenticate this tab.");
  const response = await fetch(`/api/${endpoint}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    body: JSON.stringify(payload),
    signal,
    credentials: "omit",
    redirect: "error",
    mode: "same-origin",
  });
  const data: unknown = await response.json();
  if (!response.ok) {
    const message = typeof data === "object" && data !== null && "error" in data ? String(data.error) : `Request failed (${response.status})`;
    throw new Error(message);
  }
  return data as T;
}
