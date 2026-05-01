import { useQuery } from "@tanstack/react-query";

import {
  api,
  type ConfigFinding,
  type CoverageBackend,
  type CoverageReport,
  type DivergenceRow,
  type DriftRow,
  type FreshnessRow,
  type FreshnessSeverity,
  type HealthOverview,
  type HealthState as HealthStateLabel,
  type SubsystemStatus,
} from "../lib/api";
import { fmtNum } from "../lib/format";
import { useConnKey } from "../lib/connections/useConnKey";

/// Phase8g — Health view. Surfaces the phase8a–8f health system in
/// the existing dashboard so the user notices stack-wide issues
/// before they bite. Polls each /v1/health/* endpoint via
/// TanStack Query at modest cadence (5–10 s) — the SSE stream
/// (HEALTH_STREAM_URL) is wired separately in App.tsx for the
/// topbar status pill so the view itself stays simple to test.
export function HealthView() {
  const connKey = useConnKey();
  const overviewQ = useQuery({
    queryKey: [connKey, "health", "overview"],
    queryFn: () => api.healthOverview(),
    refetchInterval: 5_000,
    refetchIntervalInBackground: true,
  });
  const freshnessQ = useQuery({
    queryKey: [connKey, "health", "freshness"],
    queryFn: () => api.healthFreshness(),
    refetchInterval: 5_000,
  });
  const divergenceQ = useQuery({
    queryKey: [connKey, "health", "divergence"],
    queryFn: () => api.healthDivergence(),
    refetchInterval: 5_000,
  });
  const versionsQ = useQuery({
    queryKey: [connKey, "health", "versions"],
    queryFn: () => api.healthVersions(),
    refetchInterval: 30_000,
  });
  const configQ = useQuery({
    queryKey: [connKey, "health", "config"],
    queryFn: () => api.healthConfig(),
    refetchInterval: 60_000,
  });
  // Phase11e §2.2 — collection / index inventory diff. 60 s
  // refetch is plenty: collections only appear when the writer
  // path runs the (rare) bootstrap or settings-version bump.
  const coverageQ = useQuery({
    queryKey: [connKey, "health", "coverage"],
    queryFn: () => api.healthCoverage(),
    refetchInterval: 60_000,
  });

  const overview = overviewQ.data;
  const freshness = freshnessQ.data ?? [];
  const divergence = divergenceQ.data ?? [];
  const versions = versionsQ.data;
  const audit = configQ.data;
  const coverage = coverageQ.data;

  return (
    <div className="view view--health">
      <OverallBanner overview={overview} />
      <CoverageSection coverage={coverage} />
      <SubsystemsGrid subsystems={overview?.subsystems ?? []} />
      <FreshnessTable rows={freshness} />
      <DivergenceTable rows={divergence} />
      <VersionDriftSection versions={versions} />
      <ConfigAuditSection findings={audit?.findings ?? []} />
    </div>
  );
}

function severityClass(sev: FreshnessSeverity | HealthStateLabel): string {
  if (sev === "ok") return "health-pill health-pill--ok";
  if (sev === "warn") return "health-pill health-pill--warn";
  if (sev === "critical" || sev === "down") return "health-pill health-pill--critical";
  if (sev === "degraded") return "health-pill health-pill--warn";
  return "health-pill";
}

function OverallBanner({ overview }: { overview?: HealthOverview }) {
  if (!overview) {
    return (
      <div className="health-banner health-banner--unknown">
        <strong>cortex-api unreachable</strong>
        <span> · waiting for /v1/health…</span>
      </div>
    );
  }
  const downCount = overview.subsystems.filter((s) => s.state === "down").length;
  const degCount = overview.subsystems.filter((s) => s.state === "degraded").length;
  const summary =
    overview.overall === "ok"
      ? `Stack healthy · ${overview.subsystems.length} subsystems`
      : overview.overall === "degraded"
        ? `${degCount} subsystem${degCount === 1 ? "" : "s"} degraded`
        : `${downCount} subsystem${downCount === 1 ? "" : "s"} down`;
  return (
    <div className={`health-banner health-banner--${overview.overall}`}>
      <span className={severityClass(overview.overall)}>{overview.overall}</span>
      <strong>{summary}</strong>
      <span className="muted"> · checked at {overview.checked_at}</span>
    </div>
  );
}

function SubsystemsGrid({ subsystems }: { subsystems: SubsystemStatus[] }) {
  if (subsystems.length === 0) {
    return <EmptySection title="Subsystems" message="no subsystem reports yet" />;
  }
  return (
    <section className="health-section">
      <h2>Subsystems</h2>
      <div className="health-grid">
        {subsystems.map((s) => (
          <SubsystemCard key={s.name} subsystem={s} />
        ))}
      </div>
    </section>
  );
}

function SubsystemCard({ subsystem }: { subsystem: SubsystemStatus }) {
  const version = (subsystem.extras?.version as { git_sha_short?: string } | undefined)
    ?.git_sha_short;
  return (
    <div className="health-card">
      <div className="health-card__head">
        <strong>{subsystem.name}</strong>
        <span className={severityClass(subsystem.state)}>{subsystem.state}</span>
      </div>
      <div className="health-card__meta mono">
        <span>v{subsystem.version ?? "?"}</span>
        {version ? <span> · sha {version}</span> : null}
        <span> · {subsystem.latency_ms}ms</span>
      </div>
      {subsystem.last_error ? (
        <div className="health-card__error">{subsystem.last_error}</div>
      ) : null}
    </div>
  );
}

function FreshnessTable({ rows }: { rows: FreshnessRow[] }) {
  // Sort by gap_seconds desc; the staler rows surface first so the
  // operator's eye lands on them.
  const sorted = [...rows].sort((a, b) => b.gap_seconds - a.gap_seconds);
  if (sorted.length === 0) {
    return <EmptySection title="Freshness" message="no freshness data yet" />;
  }
  return (
    <section className="health-section">
      <h2>Freshness</h2>
      <table className="health-table">
        <thead>
          <tr>
            <th>stage.kind</th>
            <th>gap</th>
            <th>severity</th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((r) => (
            <tr key={r.key}>
              <td className="mono">{r.key}</td>
              <td className="mono">
                {r.gap_seconds < 0 ? "—" : `${fmtNum(r.gap_seconds)}s`}
              </td>
              <td>
                <span className={severityClass(r.severity)}>{r.severity}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function DivergenceTable({ rows }: { rows: DivergenceRow[] }) {
  // Show only rows where severity != ok — `ok` rows are noise on
  // this view, the operator opens the freshness table when they
  // want the full picture.
  const interesting = rows.filter((r) => r.severity !== "ok");
  if (interesting.length === 0) {
    return (
      <EmptySection
        title="Divergence"
        message="no adjacent-stage drops detected"
      />
    );
  }
  return (
    <section className="health-section">
      <h2>Divergence</h2>
      <table className="health-table">
        <thead>
          <tr>
            <th>pair</th>
            <th>upstream</th>
            <th>downstream</th>
            <th>delta</th>
            <th>growth</th>
            <th>severity</th>
          </tr>
        </thead>
        <tbody>
          {interesting.map((r) => (
            <tr key={r.pair}>
              <td className="mono">{r.pair}</td>
              <td className="mono">{fmtNum(r.upstream)}</td>
              <td className="mono">{fmtNum(r.downstream)}</td>
              <td className="mono">{fmtNum(r.delta)}</td>
              <td className="mono">{fmtNum(r.delta_growth)}</td>
              <td>
                <span className={severityClass(r.severity)}>{r.severity}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function VersionDriftSection({
  versions,
}: {
  versions?: { all_in_sync: boolean; drift: DriftRow[]; head_sha_short: string };
}) {
  if (!versions || versions.all_in_sync) {
    return (
      <EmptySection
        title="Version drift"
        message={
          versions
            ? `all binaries match HEAD ${versions.head_sha_short}`
            : "version data unavailable"
        }
      />
    );
  }
  return (
    <section className="health-section">
      <h2>Version drift</h2>
      <p className="muted">
        Workspace HEAD: <span className="mono">{versions.head_sha_short}</span>
      </p>
      <table className="health-table">
        <thead>
          <tr>
            <th>binary</th>
            <th>running</th>
            <th>expected</th>
            <th>behind</th>
          </tr>
        </thead>
        <tbody>
          {versions.drift.map((d) => (
            <tr key={d.binary}>
              <td className="mono">{d.binary}</td>
              <td className="mono">{d.running_sha.slice(0, 7)}</td>
              <td className="mono">{d.expected_sha.slice(0, 7)}</td>
              <td className="mono">
                {d.behind_by_commits === null
                  ? `— (${d.note ?? "unreachable"})`
                  : `${d.behind_by_commits} commits`}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

/// Phase11e §2.2 — Coverage section. Renders the per-backend
/// expected/present/missing counts the auditor produces, plus a
/// collapsible row per backend listing the first ten missing
/// collection / index names so the operator sees the most
/// load-bearing gap at a glance without leaving the dashboard.
function CoverageSection({ coverage }: { coverage?: CoverageReport }) {
  if (!coverage) {
    return (
      <EmptySection
        title="Coverage"
        message="waiting for /v1/health/coverage…"
      />
    );
  }
  if (coverage.backends.length === 0) {
    return (
      <EmptySection
        title="Coverage"
        message="no Vectorizer / Meili URLs configured"
      />
    );
  }
  return (
    <section className="health-section">
      <h2>
        Coverage
        <span className={severityClass(coverage.overall_severity)} style={{ marginLeft: 8 }}>
          {coverage.overall_severity}
        </span>
      </h2>
      <p className="muted" style={{ marginTop: 0, marginBottom: 8 }}>
        {coverage.slugs.length} slug{coverage.slugs.length === 1 ? "" : "s"}
        {" × "}
        {coverage.families.length} families
        {" = "}
        {coverage.slugs.length * coverage.families.length} expected names per backend
      </p>
      <div className="health-grid">
        {coverage.backends.map((b) => (
          <CoverageCard key={b.backend} backend={b} />
        ))}
      </div>
    </section>
  );
}

function CoverageCard({ backend }: { backend: CoverageBackend }) {
  const ratio =
    backend.expected_count > 0
      ? Math.round((backend.present_count / backend.expected_count) * 100)
      : 0;
  return (
    <div className="health-card">
      <div className="health-card__head">
        <strong>{backend.backend}</strong>
        <span className={severityClass(backend.severity)}>{backend.severity}</span>
      </div>
      <div className="health-card__meta mono">
        <span>
          {fmtNum(backend.present_count)} / {fmtNum(backend.expected_count)}
        </span>
        <span> · {ratio}% present</span>
        {backend.unexpected_count > 0 ? (
          <span> · {fmtNum(backend.unexpected_count)} orphan</span>
        ) : null}
      </div>
      {backend.error ? (
        <div className="health-card__error">{backend.error}</div>
      ) : null}
      {backend.missing.length > 0 ? (
        <details style={{ marginTop: 8, fontSize: 11 }}>
          <summary className="muted" style={{ cursor: "pointer" }}>
            {fmtNum(backend.missing_count)} missing
            {backend.missing.length > 10 ? " (showing first 10)" : ""}
          </summary>
          <ul
            className="mono"
            style={{ margin: "6px 0 0", paddingLeft: 18, lineHeight: 1.4 }}
          >
            {backend.missing.slice(0, 10).map((name) => (
              <li key={name}>{name}</li>
            ))}
          </ul>
        </details>
      ) : null}
    </div>
  );
}

function ConfigAuditSection({ findings }: { findings: ConfigFinding[] }) {
  const interesting = findings.filter((f) => f.severity !== "ok");
  if (interesting.length === 0) {
    return (
      <EmptySection
        title="Config audit"
        message="every config surface aligned"
      />
    );
  }
  return (
    <section className="health-section">
      <h2>Config audit</h2>
      <table className="health-table">
        <thead>
          <tr>
            <th>severity</th>
            <th>source</th>
            <th>message</th>
          </tr>
        </thead>
        <tbody>
          {interesting.map((f, idx) => (
            <tr key={`${f.source}-${idx}`}>
              <td>
                <span className={severityClass(f.severity)}>{f.severity}</span>
              </td>
              <td className="mono">{f.source}</td>
              <td>{f.message}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function EmptySection({ title, message }: { title: string; message: string }) {
  return (
    <section className="health-section">
      <h2>{title}</h2>
      <p className="muted">{message}</p>
    </section>
  );
}
