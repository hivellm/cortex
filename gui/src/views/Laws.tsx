import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "../atoms/Icon";
import { SeverityBar } from "../atoms/SeverityBar";
import { Tag } from "../atoms/Tag";
import { api, type LawRow, type ViolationRow } from "../lib/api";

export function LawsView() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const lawsQ = useQuery({
    queryKey: ["laws"],
    queryFn: () => api.laws(),
    refetchInterval: 30_000,
  });
  const violationsQ = useQuery({
    queryKey: ["violations"],
    queryFn: () => api.violations(),
    refetchInterval: 10_000,
    refetchIntervalInBackground: true,
  });

  const lawsRaw = lawsQ.data ?? [];
  const violations = violationsQ.data ?? [];

  // Sort by violation rate descending so the most-active laws bubble
  // to the top — explicit so the UI does not depend on the backend's
  // BTreeMap iteration order.
  const laws = useMemo(
    () => [...lawsRaw].sort((a, b) => b.violations_7d - a.violations_7d),
    [lawsRaw],
  );

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Laws</h1>
          <p className="view__subtitle">
            Spec-13 catalogue + spec-14 enforcement. The catalogue endpoint
            (<span className="mono">/v1/dashboard/laws</span>) returns empty until spec-13 ships;
            <span className="mono"> /v1/dashboard/violations</span> tracks observed
            <span className="mono"> kind=law_violation</span> envelopes.
          </p>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
        <Stat label="Active laws" value={String(laws.length)} sub="spec-13 catalogue" />
        <Stat
          label="Critical · 7d"
          value={String(violations.filter((v) => v.action === "blocked").length)}
          sub={`${violations.length} total`}
        />
        <Stat
          label="Annotated · 7d"
          value={String(violations.filter((v) => v.action === "annotated").length)}
          sub="recorded but not blocked"
        />
      </div>

      <section style={{ marginTop: 16 }}>
        <h2 style={{ fontSize: 13, color: "var(--fg-2)", marginBottom: 8 }}>Catalogue</h2>
        {lawsQ.error ? (
          <Empty msg="cortex-api unreachable." />
        ) : lawsQ.isLoading ? (
          <Empty msg="Loading laws…" />
        ) : laws.length === 0 ? (
          <Empty msg="The law catalogue is empty. Spec-13 will populate it." />
        ) : (
          <div className="law-table">
            <div className="law-row law-row--header">
              <span>ID</span>
              <span>Title</span>
              <span>Severity</span>
              <span>Action</span>
              <span>Scope</span>
              <span style={{ textAlign: "right" }}>Rate · 7d</span>
            </div>
            {laws.map((law) => (
              <div
                key={law.id}
                className={`law-row ${selectedId === law.id ? "is-active" : ""}`}
                onClick={() => setSelectedId(law.id)}
                role="button"
                tabIndex={0}
              >
                <span className="mono" style={{ color: "var(--accent)", fontSize: 11 }}>
                  {law.id}
                </span>
                <span className="law-row__title">{law.title}</span>
                <span className="law-row__sev">
                  <SeverityBar severity={law.severity} />
                  <span
                    style={{
                      marginLeft: 6,
                      color:
                        law.severity === "critical"
                          ? "var(--critical)"
                          : law.severity === "notable" || law.severity === "warn"
                            ? "var(--warn)"
                            : "var(--info)",
                      fontFamily: "var(--font-mono)",
                      fontSize: 10.5,
                    }}
                  >
                    {law.severity}
                  </span>
                </span>
                <span>
                  {law.blocked ? (
                    <Tag tone="critical">block</Tag>
                  ) : (
                    <Tag>observe</Tag>
                  )}
                </span>
                <span className="muted mono" style={{ fontSize: 10.5 }}>
                  {law.scope}
                </span>
                <span
                  className="mono tabular"
                  style={{ fontSize: 10.5, textAlign: "right" }}
                >
                  {law.violations_7d}
                  <span className="muted"> / {law.applies}</span>
                </span>
              </div>
            ))}
          </div>
        )}
      </section>

      <section style={{ marginTop: 16 }}>
        <h2 style={{ fontSize: 13, color: "var(--fg-2)", marginBottom: 8 }}>
          Recent violations
        </h2>
        {violationsQ.error ? (
          <Empty msg="cortex-api unreachable." />
        ) : violationsQ.isLoading ? (
          <Empty msg="Loading violations…" />
        ) : violations.length === 0 ? (
          <Empty msg="No violations observed yet." />
        ) : (
          <div className="violation-list">
            {violations.map((v) => (
              <div key={v.id} className="violation-row">
                <span className="mono" style={{ color: "var(--accent)" }}>
                  {v.id}
                </span>
                <Tag tone={v.action === "blocked" ? "critical" : "warn"}>{v.action}</Tag>
                <span className="muted">
                  {v.law_id ?? "—"} · {v.repo ?? "—"} · {v.at}
                </span>
                <pre className="violation-evidence">{v.evidence}</pre>
              </div>
            ))}
          </div>
        )}
      </section>

      <LawInspector
        law={laws.find((l) => l.id === selectedId) ?? null}
        violations={violations.filter((v) => v.law_id === selectedId)}
        onClose={() => setSelectedId(null)}
      />
    </div>
  );
}

function LawInspector({
  law,
  violations,
  onClose,
}: {
  law: LawRow | null;
  violations: ViolationRow[];
  onClose: () => void;
}) {
  const open = !!law;
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!law) {
    return (
      <>
        <div className={`inspector-backdrop ${open ? "is-open" : ""}`} onClick={onClose} />
        <aside className={`inspector ${open ? "is-open" : ""}`} />
      </>
    );
  }

  const severityColor =
    law.severity === "critical"
      ? "var(--critical)"
      : law.severity === "notable" || law.severity === "warn"
        ? "var(--warn)"
        : "var(--info)";
  const severityBg =
    law.severity === "critical"
      ? "var(--critical-soft)"
      : law.severity === "notable" || law.severity === "warn"
        ? "var(--warn-soft)"
        : "var(--info-soft)";

  const yamlBody = `---
id: ${law.id}
title: ${law.title}
severity: ${law.severity}
applies_to: [${law.scope
    .split(",")
    .map((s) => `"${s.trim()}"`)
    .join(", ")}]
detector: ${law.detector || "(unspecified)"}
remediation: |
  ${law.remediation || "(none recorded)"}
---
The model MUST follow this rule unless the user has
explicitly authorized an exception in this session.`;

  return (
    <>
      <div className={`inspector-backdrop ${open ? "is-open" : ""}`} onClick={onClose} />
      <aside className={`inspector ${open ? "is-open" : ""}`}>
        <div className="inspector__head">
          <span
            style={{
              width: 26,
              height: 26,
              display: "grid",
              placeItems: "center",
              borderRadius: 4,
              background: severityBg,
              color: severityColor,
              border: `1px solid ${severityColor}`,
            }}
          >
            <Icon name={law.blocked ? "block" : "alert"} size={14} />
          </span>
          <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0 }}>
            <span className="inspector__title">{law.id}</span>
            <span className="inspector__id">
              {law.severity} · {law.blocked ? "blocking" : "observational"}
            </span>
          </div>
          <button
            className="icon-btn"
            onClick={onClose}
            style={{ marginLeft: "auto" }}
            aria-label="Close inspector"
          >
            <Icon name="close" size={15} />
          </button>
        </div>
        <div className="inspector__body">
          <div className="inspector__section">
            <div
              style={{
                fontSize: 14,
                color: "var(--fg-0)",
                fontWeight: 600,
                marginBottom: 6,
                letterSpacing: "-0.01em",
              }}
            >
              {law.title}
            </div>
            <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
              <Tag tone={law.severity === "critical" ? "critical" : law.severity === "warn" || law.severity === "notable" ? "warn" : "info"}>
                {law.severity}
              </Tag>
              {law.blocked ? <Tag tone="critical">block</Tag> : <Tag>observe</Tag>}
              <Tag tone="solid">{law.scope}</Tag>
            </div>
          </div>
          <div className="inspector__section">
            <div className="inspector__section-label">Definition (laws/{law.id}.md)</div>
            <pre className="code-block">{yamlBody}</pre>
          </div>
          <div className="inspector__section">
            <div className="inspector__section-label">7-day stats</div>
            <dl className="kv-list">
              <dt>applies</dt>
              <dd className="mono tabular">{law.applies} eligible events</dd>
              <dt>violations</dt>
              <dd className="mono tabular">{law.violations_7d}</dd>
              <dt>rate</dt>
              <dd className="mono tabular">{law.rate.toFixed(2)} per 1k</dd>
              <dt>action</dt>
              <dd className="mono">
                {law.blocked ? "PreToolUse block" : "PostToolUse annotate"}
              </dd>
            </dl>
          </div>
          {violations.length > 0 ? (
            <div className="inspector__section">
              <div className="inspector__section-label">Recent violations</div>
              {violations.map((v) => (
                <div key={v.id} className="violation-card">
                  <div className="violation-card__head">
                    <span
                      className="mono"
                      style={{ fontSize: 11, color: "var(--accent)", fontWeight: 600 }}
                    >
                      {v.id}
                    </span>
                    <Tag tone={v.action === "blocked" ? "critical" : "warn"}>{v.action}</Tag>
                    <span
                      className="mono"
                      style={{ fontSize: 10.5, color: "var(--fg-3)", marginLeft: "auto" }}
                    >
                      {v.at}
                    </span>
                  </div>
                  <div style={{ fontSize: 11.5, color: "var(--fg-2)", marginBottom: 4 }}>
                    {v.repo ?? "—"}
                  </div>
                  <pre
                    className="code-block"
                    style={{ fontSize: 11, padding: "6px 8px", marginTop: 6 }}
                  >
                    {v.evidence}
                  </pre>
                  {v.remediation ? (
                    <div style={{ fontSize: 11, color: "var(--fg-3)", marginTop: 6 }}>
                      {v.remediation}
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
        </div>
      </aside>
    </>
  );
}

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="stat">
      <div className="stat__label">{label}</div>
      <div className="stat__value tabular">{value}</div>
      {sub ? <div className="stat__delta">{sub}</div> : null}
    </div>
  );
}

function Empty({ msg }: { msg: string }) {
  return (
    <div
      style={{
        padding: 24,
        border: "1px dashed var(--border)",
        borderRadius: "var(--radius-md)",
        color: "var(--fg-3)",
        textAlign: "center",
        fontSize: 12,
      }}
    >
      {msg}
    </div>
  );
}
