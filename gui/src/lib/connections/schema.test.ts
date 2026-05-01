/**
 * schema.test.ts — phase3 §8 unit coverage for the persisted-state
 * validator.
 */

import { describe, expect, it } from "vitest";

import { validateConnectionsState } from "./schema";
import { LOCAL_CONNECTION_ID } from "./types";

describe("validateConnectionsState", () => {
  it("accepts a minimal valid document", () => {
    const r = validateConnectionsState({
      activeId: LOCAL_CONNECTION_ID,
      connections: [
        {
          id: LOCAL_CONNECTION_ID,
          label: "Local",
          baseUrl: "http://127.0.0.1:17000",
          auth: { kind: "none" },
          color: "#22c55e",
          createdAt: 0,
        },
      ],
    });
    expect(r.ok).toBe(true);
  });

  it("rejects a document missing the local connection", () => {
    const r = validateConnectionsState({
      activeId: "abc",
      connections: [
        {
          id: "abc",
          label: "Staging",
          baseUrl: "https://stage.example.com",
          auth: { kind: "none" },
          color: "#000000",
          createdAt: 1,
        },
      ],
    });
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.issues.some((i) => i.includes("local"))).toBe(true);
    }
  });

  it("rejects connections with an invalid base URL", () => {
    const r = validateConnectionsState({
      activeId: LOCAL_CONNECTION_ID,
      connections: [
        {
          id: LOCAL_CONNECTION_ID,
          label: "Local",
          baseUrl: "not-a-url",
          auth: { kind: "none" },
          color: "#22c55e",
          createdAt: 0,
        },
      ],
    });
    expect(r.ok).toBe(false);
  });

  it("rejects bearer auth missing token", () => {
    const r = validateConnectionsState({
      activeId: LOCAL_CONNECTION_ID,
      connections: [
        {
          id: LOCAL_CONNECTION_ID,
          label: "Local",
          baseUrl: "http://127.0.0.1:17000",
          auth: { kind: "bearer" },
          color: "#22c55e",
          createdAt: 0,
        },
      ],
    });
    expect(r.ok).toBe(false);
  });

  it("rejects duplicated connection ids", () => {
    const r = validateConnectionsState({
      activeId: LOCAL_CONNECTION_ID,
      connections: [
        {
          id: LOCAL_CONNECTION_ID,
          label: "Local",
          baseUrl: "http://127.0.0.1:17000",
          auth: { kind: "none" },
          color: "#22c55e",
          createdAt: 0,
        },
        {
          id: LOCAL_CONNECTION_ID,
          label: "Dup",
          baseUrl: "http://127.0.0.1:17001",
          auth: { kind: "none" },
          color: "#000000",
          createdAt: 1,
        },
      ],
    });
    expect(r.ok).toBe(false);
  });

  it("strips trailing slashes from baseUrl", () => {
    const r = validateConnectionsState({
      activeId: LOCAL_CONNECTION_ID,
      connections: [
        {
          id: LOCAL_CONNECTION_ID,
          label: "Local",
          baseUrl: "http://127.0.0.1:17000///",
          auth: { kind: "none" },
          color: "#22c55e",
          createdAt: 0,
        },
      ],
    });
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.value.connections[0].baseUrl).toBe("http://127.0.0.1:17000");
    }
  });

  it("falls back to local when activeId references a missing connection", () => {
    const r = validateConnectionsState({
      activeId: "ghost",
      connections: [
        {
          id: LOCAL_CONNECTION_ID,
          label: "Local",
          baseUrl: "http://127.0.0.1:17000",
          auth: { kind: "none" },
          color: "#22c55e",
          createdAt: 0,
        },
      ],
    });
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.value.activeId).toBe(LOCAL_CONNECTION_ID);
    }
  });

  it("rejects non-object root input", () => {
    const r = validateConnectionsState("not an object");
    expect(r.ok).toBe(false);
  });

  it("accepts basic auth with username + password", () => {
    const r = validateConnectionsState({
      activeId: LOCAL_CONNECTION_ID,
      connections: [
        {
          id: LOCAL_CONNECTION_ID,
          label: "Local",
          baseUrl: "http://127.0.0.1:17000",
          auth: { kind: "none" },
          color: "#22c55e",
          createdAt: 0,
        },
        {
          id: "remote",
          label: "Remote",
          baseUrl: "https://cortex.example.com",
          auth: { kind: "basic", username: "u", password: "p" },
          color: "#3b82f6",
          createdAt: 100,
        },
      ],
    });
    expect(r.ok).toBe(true);
  });
});
