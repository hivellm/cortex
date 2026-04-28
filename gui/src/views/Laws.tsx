import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
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
  const trustQ = useQuery({
    queryKey: ["trust"],
    queryFn: () => api.trust(),
    refetchInterval: 60_000,
  });
  const trust = trustQ.data;

  // Sort by violation rate descending so the most-active laws bubble
  // to the top — explicit so the UI does not depend on the backend's
  // BTreeMap iteration order.
  const laws = useMemo(
    () => [...lawsRaw].sort((a, b) => b.violations_7d - a.violations_7d),
    [lawsRaw],
  );

  // Stats derived from the current law set + violation stream. Honest
  // numbers — when spec-13/spec-14 haven't shipped these read as 0
  // rather than mocked values, so the dashboard never fakes activity.
  const blocking = laws.filter((l) => l.blocked).length;
  const observational = laws.filter((l) => !l.blocked).length;
  const blocked7d = violations.filter((v) => v.action === "blocked").length;
  const flagged7d = violations.filter((v) => v.action !== "blocked").length;
  const falseBlockPct =
    blocked7d === 0 ? 0 : (violations.filter((v) => v.action === "annotated").length / blocked7d) * 100;
  const trustRange = useMemo(() => {
    if (!trust || trust.models.length === 0 || trust.repos.length === 0) {
      return null;
    }
    const flat: number[] = [];
    for (const m of trust.models) {
      const row = trust.scores[m] ?? {};
      for (const r of trust.repos) {
        const s = row[r];
        if (typeof s === "number") flat.push(s);
      }
    }
    if (flat.length === 0) return null;
    return { min: Math.min(...flat), max: Math.max(...flat) };
  }, [trust]);

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Law dashboard</h1>
          <p className="view__subtitle">
            Codified rules · graduated punishment · per-(model, repo) trust score
          </p>
        </div>
        <div className="view__actions">
          <button className="btn" type="button" disabled title="Lints the law catalogue against the spec-13 schema. Available once authoring lands.">
            <Icon name="external" size={13} /> Lint laws
          </button>
          <button className="btn btn--primary" type="button" disabled title="Opens the law-authoring split pane. Available once spec-13 authoring lands.">
            <Icon name="law" size={13} /> Author new law
          </button>
        </div>
      </div>

      <div className="stats-grid" style={{ gridTemplateColumns: "repeat(4, 1fr)" }}>
        <Stat
          label={
            <>
              <Icon name="block" size={12} /> Blocking laws
            </>
          }
          labelColor="var(--critical)"
          value={String(blocking)}
          sub={`${blocked7d} fired · 7d`}
        />
        <Stat
          label={
            <>
              <Icon name="alert" size={12} /> Observational
            </>
          }
          value={String(observational)}
          sub={`${flagged7d} events flagged · 7d`}
        />
        <Stat
          label="False-block rate"
          value={`${falseBlockPct.toFixed(1)}%`}
          sub={blocked7d > 0 ? "annotated ÷ blocked · 7d" : "no blocks observed yet"}
        />
        <Stat
          label="Trust score · range"
          value={trustRange ? `${trustRange.min.toFixed(2)} – ${trustRange.max.toFixed(2)}` : "—"}
          sub={
            trustRange
              ? `${trust?.models.length ?? 0} models × ${trust?.repos.length ?? 0} repos`
              : "spec-14 derivation pending"
          }
        />
      </div>

      <div className="card" style={{ marginTop: 18, marginBottom: 18 }}>
        <div className="card__head">
          <span className="card__title">Active laws</span>
          <span className="card__sub">
            {laws.length} {laws.length === 1 ? "law" : "laws"} · sorted by violation rate
          </span>
        </div>
        {lawsQ.error ? (
          <div className="card__body">
            <Empty msg="cortex-api unreachable." />
          </div>
        ) : lawsQ.isLoading ? (
          <div className="card__body">
            <Empty msg="Loading laws…" />
          </div>
        ) : laws.length === 0 ? (
          <div className="card__body">
            <Empty msg="The law catalogue is empty. Spec-13 will populate it." />
          </div>
        ) : (
          <div>
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
                <span className="law-row__id">{law.id}</span>
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
                <span className="mono" style={{ fontSize: 11, color: "var(--fg-2)" }}>
                  {law.scope}
                </span>
                <span
                  className="law-row__rate"
                  style={{ textAlign: "right" }}
                >
                  {law.violations_7d}
                  <span className="muted"> / {law.applies}</span>
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="card" style={{ marginBottom: 18 }}>
        <div className="card__head">
          <span className="card__title">Trust score · per (model, repo)</span>
          <span className="card__sub">recomputed nightly · 30-day rolling</span>
        </div>
        <div className="card__body">
          {trustQ.isLoading ? (
            <Empty msg="Loading trust matrix…" />
          ) : !trust ? (
            <Empty msg="Trust matrix unavailable." />
          ) : trust.source === "stub_until_spec14" ? (
            <Empty msg="Spec-14 trust derivation has not run yet. Empty until model × repo scores are stored under cortex.events.violations." />
          ) : trust.models.length === 0 || trust.repos.length === 0 ? (
            <Empty msg="Trust derivation reported no signal yet — no observed (model, repo) pairs." />
          ) : (
            <TrustGrid trust={trust} />
          )}
        </div>
      </div>

      <section style={{ marginBottom: 16 }}>
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

/// Heatmap-style grid: rows are models, columns are repos. Cell
/// shading runs through the same `oklch` ramp the design uses
/// (gui/assets/views-mid.jsx lines 219-227): low scores red, mid
/// amber, high green, modulated by score-driven alpha so the eye
/// reads density quickly.
function TrustGrid({ trust }: { trust: import("../lib/api").TrustMatrix }) {
  const { models, repos, scores } = trust;
  const visibleRepos = repos.slice(0, 5);
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: `180px repeat(${visibleRepos.length}, 1fr)`,
        gap: 6,
        fontSize: 11.5,
        fontFamily: "var(--font-mono)",
      }}
    >
      <div />
      {visibleRepos.map((r) => (
        <div key={r} style={{ color: "var(--fg-3)", padding: 4 }}>
          {r}
        </div>
      ))}
      {models.map((m) => (
        <Fragment key={m}>
          <div style={{ color: "var(--fg-1)", padding: 6, fontSize: 11 }}>{m}</div>
          {visibleRepos.map((r) => {
            const score = scores[m]?.[r];
            if (typeof score !== "number") {
              return (
                <div
                  key={r}
                  style={{
                    padding: "8px 10px",
                    borderRadius: 4,
                    color: "var(--fg-4, var(--fg-3))",
                    textAlign: "center",
                    border: "1px solid var(--border-soft)",
                  }}
                >
                  —
                </div>
              );
            }
            const hue = 25 + score * 110;
            const bg = `oklch(0.42 0.10 ${hue} / ${0.35 + score * 0.5})`;
            const fg =
              score > 0.85
                ? "oklch(0.95 0.05 155)"
                : score > 0.75
                  ? "oklch(0.95 0.10 90)"
                  : "oklch(0.95 0.10 25)";
            return (
              <div
                key={r}
                style={{
                  padding: "8px 10px",
                  background: bg,
                  borderRadius: 4,
                  color: fg,
                  fontVariantNumeric: "tabular-nums",
                  textAlign: "center",
                  border: "1px solid var(--border-soft)",
                }}
              >
                {score.toFixed(2)}
              </div>
            );
          })}
        </Fragment>
      ))}
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

function Stat({
  label,
  value,
  sub,
  labelColor,
}: {
  label: ReactNode;
  value: string;
  sub?: string;
  labelColor?: string;
}) {
  return (
    <div className="stat">
      <div className="stat__label" style={labelColor ? { color: labelColor } : undefined}>
        {label}
      </div>
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
