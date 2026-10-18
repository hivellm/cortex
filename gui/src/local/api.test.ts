import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { bootstrapLocalSession, initialRepository, localRequest, watchLocalStartupNavigation } from "./api";

const token = "a".repeat(64);
beforeEach(() => {
  document.head.innerHTML = '<meta name="cortex-runtime" content="standalone">';
  sessionStorage.clear(); localStorage.clear();
  history.replaceState(null, "", "/");
  bootstrapLocalSession();
});
afterEach(() => { vi.unstubAllGlobals(); });

it("consumes token before requests, preserves exact scope and only uses session storage", async () => {
  history.replaceState(null, "", `/#token=${token}&repository=%20Cortex%20`);
  bootstrapLocalSession();
  expect(location.hash).toBe("");
  expect(initialRepository).toBe(" Cortex ");
  expect(localStorage.length).toBe(0);
  const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ records: [] }) });
  vi.stubGlobal("fetch", fetcher);
  await localRequest("browse", { repository: initialRepository });
  expect(fetcher).toHaveBeenCalledWith("/api/browse", expect.objectContaining({ method: "POST", redirect: "error", mode: "same-origin", credentials: "omit", headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` }, body: '{"repository":" Cortex "}' }));
  bootstrapLocalSession();
  expect(initialRepository).toBe(" Cortex ");
  await localRequest("status", {});
});

it("invalid replacement token clears previous credentials and strips fragment", async () => {
  history.replaceState(null, "", `/#token=${token}&repository=Cortex`); bootstrapLocalSession();
  history.replaceState(null, "", "/#token=invalid&repository=cortex"); bootstrapLocalSession();
  expect(location.hash).toBe("");
  expect(sessionStorage.getItem("cortex.local.token")).toBeNull();
  await expect(localRequest("status", {})).rejects.toThrow("startup URL");
});

it.each(["https://example.com", "//example.com", "../status", "status?token=x", "/api/status", "audit/%2e%2e"]) ("rejects nonlocal endpoint %s", async (endpoint) => {
  await expect(localRequest(endpoint, {})).rejects.toThrow("Invalid local endpoint");
});

it("does not touch legacy credentials or fragments outside standalone", () => {
  document.head.innerHTML = "";
  sessionStorage.clear();
  localStorage.setItem("legacy", "existing");
  history.replaceState(null, "", "/#token=legacy"); bootstrapLocalSession();
  expect(location.hash).toBe("#token=legacy");
  expect(localStorage.getItem("legacy")).toBe("existing");
  expect(sessionStorage.length).toBe(0);
});

it("works with in-memory credentials when session storage is unavailable", async () => {
  history.replaceState(null, "", `/#token=${token}&repository=Cortex`);
  const spy = vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => { throw new Error("disabled"); });
  bootstrapLocalSession(); spy.mockRestore();
  const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) }); vi.stubGlobal("fetch", fetcher);
  await localRequest("status", {});
  expect(fetcher).toHaveBeenCalledOnce();
  expect(location.hash).toBe("");
});

it("reloads same-document startup navigation before consuming its token, even with storage blocked", async () => {
  history.replaceState(null, "", `/#token=${token}&repository=Cortex`);
  bootstrapLocalSession();
  const replacement = "b".repeat(64);
  const fragment = `#token=${replacement}&repository=cortex`;
  const reload = vi.fn(() => { expect(location.hash).toBe(fragment); });
  const stop = watchLocalStartupNavigation(reload);
  const storage = vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => { throw new Error("disabled"); });
  try {
    history.replaceState(null, "", `/${fragment}`);
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    expect(reload).toHaveBeenCalledOnce();
    expect(storage).not.toHaveBeenCalled();
    bootstrapLocalSession();
    expect(initialRepository).toBe("cortex");
    expect(location.hash).toBe("");
    const fetcher = vi.fn().mockResolvedValue({ ok: true, json: async () => ({}) });
    vi.stubGlobal("fetch", fetcher);
    await localRequest("status", {});
    expect(fetcher).toHaveBeenCalledWith("/api/status", expect.objectContaining({ headers: expect.objectContaining({ Authorization: `Bearer ${replacement}` }) }));
  } finally { storage.mockRestore(); stop(); }
});

it("ignores ordinary fragments, malformed tokens and legacy navigation; listener can be removed", () => {
  const reload = vi.fn();
  const stop = watchLocalStartupNavigation(reload);
  for (const fragment of ["#memory", "#repository=Cortex", "#token=invalid"]) {
    history.replaceState(null, "", `/${fragment}`);
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  }
  document.head.innerHTML = "";
  history.replaceState(null, "", `/#token=${token}`);
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  expect(reload).not.toHaveBeenCalled();
  document.head.innerHTML = '<meta name="cortex-runtime" content="standalone">';
  stop();
  window.dispatchEvent(new HashChangeEvent("hashchange"));
  expect(reload).not.toHaveBeenCalled();
});
