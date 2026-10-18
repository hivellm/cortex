import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { DetailDrawer } from "../atoms/DetailDrawer";
import { Icon } from "../atoms/Icon";
import { Tag } from "../atoms/Tag";
import { localRequest } from "./api";
import type { AuditRecord, AuditReference, FeedbackEntry, FeedbackRequest, LocalRecord, LocalView } from "./types";
import "./workspace.css";

type Props = { view: LocalView; repository: string; searchText: string; onSearch: (text: string) => void; onNavigate: (view: LocalView) => void };
type GraphData = { records: LocalRecord[]; edges: { source_id: string; target_id: string; relation: string }[]; truncated: boolean };
type Job = { prepared: { record_id: string; revision: number }; status: string; attempts: number; known_input_tokens: number; known_output_tokens: number; unknown_usage_attempts: number };
const titles: Record<LocalView, string> = { timeline: "Timeline", memory: "Memory browser", decisions: "Decision register", classifications: "Classifications", graph: "Graph explorer", health: "Local health", audit: "Delivery audit", feedback: "Feedback" };
const pretty = (value: unknown) => JSON.stringify(value, null, 2);
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);

export function LocalWorkspace(props: Props) {
  return <Workspace key={`${props.repository}:${props.view}`} {...props} />;
}

function Workspace({ view, repository, searchText, onSearch, onNavigate }: Props) {
  const [rows, setRows] = useState<LocalRecord[]>([]);
  const [auditRows, setAuditRows] = useState<{ record: AuditRecord; archived: boolean }[]>([]);
  const [feedbackRows, setFeedbackRows] = useState<FeedbackEntry[]>([]);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [summary, setSummary] = useState<unknown>(null);
  const [inventory, setInventory] = useState<Record<string, number> | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [loading, setLoading] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [cursors, setCursors] = useState<(string | null)[]>([null]);
  const [next, setNext] = useState<string | null>(null);
  const [cursorSearch, setCursorSearch] = useState(searchText);
  // Header search changes arrive as props; reset before request effects commit.
  if (cursorSearch !== searchText) {
    setCursorSearch(searchText);
    setCursors([null]);
    setNext(null);
  }
  const [inactive, setInactive] = useState(false);
  const [ranking, setRanking] = useState(false);
  const [kind, setKind] = useState("");
  const [archived, setArchived] = useState(false);
  const [selected, setSelected] = useState<LocalRecord | null>(null);
  const [historical, setHistorical] = useState<LocalRecord | null>(null);
  const [revision, setRevision] = useState(1);
  const [graph, setGraph] = useState<GraphData | null>(null);
  const [audit, setAudit] = useState<AuditRecord | null>(null);
  const [feedback, setFeedback] = useState<FeedbackEntry | null>(null);
  const [editor, setEditor] = useState<{ original: LocalRecord | null } | null>(null);
  const [target, setTarget] = useState<FeedbackRequest | null>(null);
  const [saving, setSaving] = useState(false);
  const [explanations, setExplanations] = useState<Record<string, unknown>>({});
  const lifecycle = useRef(new AbortController());
  const selection = useRef(0);
  const frozenFeedback = useRef<FeedbackRequest | null>(null);
  useEffect(() => { const controller = new AbortController(); lifecycle.current = controller; return () => controller.abort(); }, []);
  const request = <T,>(endpoint: string, payload: object) => localRequest<T>(endpoint, { repository, ...payload }, lifecycle.current.signal);
  const report = (e: unknown) => { if (!lifecycle.current.signal.aborted) setError(errorText(e)); };
  const resetPage = () => { setCursors([null]); setNext(null); };

  useEffect(() => {
    const controller = new AbortController();
    if (view !== "timeline" || !repository) return () => controller.abort();
    void localRequest<Record<string, number>>("status", { repository }, controller.signal)
      .then(value => { if (!controller.signal.aborted) setInventory(value); })
      .catch(e => { if (!controller.signal.aborted) setError(errorText(e)); });
    return () => controller.abort();
  }, [repository, view, refresh]);

  useEffect(() => {
    const controller = new AbortController();
    if (!repository) return () => controller.abort();
    setLoading(true); setError(""); setNext(null);
    const call = <T,>(endpoint: string, payload: object = {}) => localRequest<T>(endpoint, { repository, ...payload }, controller.signal);
    async function load() {
      const cursor = cursors[cursors.length - 1];
      if (view === "health") setSummary(await call("status"));
      else if (view === "classifications") {
        const page = await call<{ jobs: Job[]; malformed: number; incomplete: boolean }>("queue", { limit: 100 });
        if (!controller.signal.aborted) { setJobs(page.jobs); setSummary({ malformed: page.malformed, incomplete: page.incomplete }); }
      } else if (view === "audit") {
        const [page, metrics] = await Promise.all([call<{ entries: typeof auditRows; next_cursor: string | null }>("audit/list", { limit: 20, cursor, include_archived: archived, ...(kind ? { kind } : {}) }), call("audit/metrics", { include_archived: archived, limit: 10000 })]);
        if (!controller.signal.aborted) { setAuditRows(page.entries); setNext(page.next_cursor); setSummary(metrics); }
      } else if (view === "feedback") {
        const page = await call<{ entries: FeedbackEntry[]; next_cursor: string | null }>("feedback/list", { limit: 20, cursor });
        if (!controller.signal.aborted) { setFeedbackRows(page.entries); setNext(page.next_cursor); }
      } else if (searchText.trim()) {
        const hits = await call<{ record: LocalRecord; feedback?: unknown }[]>("search", { text: searchText.trim(), limit: 100, include_inactive: inactive, ...(ranking ? { feedback: true } : {}) });
        if (!controller.signal.aborted) { setRows(hits.map(h => h.record).filter(r => view === "decisions" ? r.kind === "decision" : !kind || r.kind === kind)); setExplanations(Object.fromEntries(hits.filter(h => h.feedback).map(h => [h.record.id, h.feedback]))); }
      } else {
        const page = await call<{ records: LocalRecord[]; next_cursor: string | null }>("browse", { limit: 50, cursor, include_inactive: inactive, ...(view === "decisions" ? { kind: "decision" } : kind ? { kind } : {}) });
        if (!controller.signal.aborted) { setRows(page.records); setNext(page.next_cursor); setExplanations({}); }
      }
    }
    void load().catch(e => { if (!controller.signal.aborted) setError(errorText(e)); }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  // Each filter is part of this scoped request generation.
  }, [repository, view, searchText, inactive, ranking, kind, archived, cursors, refresh]);

  async function openRecord(id: string, reference?: AuditReference) {
    const generation = ++selection.current;
    try {
      const record = await request<LocalRecord>("get", { id, ...(reference ? { revision: reference.revision } : {}) });
      if (lifecycle.current.signal.aborted || generation !== selection.current) return;
      if (reference && (reference.repository !== repository || record.repository !== repository || record.id !== reference.id || record.revision !== reference.revision || record.content_sha256 !== reference.content_sha256)) throw new Error("Historical source does not match the exact audit reference.");
      setSelected(record); setHistorical(null); setRevision(record.revision); setGraph(null); setAudit(null); setFeedback(null);
      if (view === "graph") { setSelected(null); await loadGraph(record); }
    } catch (e) { report(e); }
  }
  async function loadGraph(record: LocalRecord) {
    const generation = selection.current;
    try { const value = await request<GraphData>("graph", { id: record.id, depth: 2, limit: 30 }); if (!lifecycle.current.signal.aborted && generation === selection.current) setGraph(value); } catch (e) { report(e); }
  }
  async function openAudit(id: string) {
    const generation = ++selection.current;
    try { const value = await request<AuditRecord>("audit/get", { id }); if (!lifecycle.current.signal.aborted && generation === selection.current) { setAudit(value); setSelected(null); setFeedback(null); } } catch (e) { report(e); }
  }
  async function openFeedback(id: string) {
    const generation = ++selection.current;
    try { const value = await request<FeedbackEntry>("feedback/get", { id }); if (!lifecycle.current.signal.aborted && generation === selection.current) { setFeedback(value); setAudit(null); setSelected(null); } } catch (e) { report(e); }
  }
  function judge(delivery: AuditRecord, reference?: AuditReference) {
    frozenFeedback.current = null;
    setTarget({ id: `feedback-${crypto.randomUUID()}`, repository, query_id: delivery.payload.content.query_id, delivery_id: delivery.id, reference, source: { kind: "local_web", reference: "dashboard", actor: "user" }, evidence: "explicit", judgment: "unknown", comment: "" });
    setAudit(null); setError("");
  }
  async function saveFeedback(event: FormEvent) {
    event.preventDefault(); if (!target || saving) return;
    // Retry the exact immutable request after an uncertain transport failure.
    const payload = frozenFeedback.current ?? structuredClone(target); frozenFeedback.current = payload;
    setSaving(true); setError("");
    try { const result = await request<{ entry: FeedbackEntry }>("feedback/record", payload); if (!lifecycle.current.signal.aborted) { setTarget(null); frozenFeedback.current = null; setFeedback(result.entry); setNotice("Declared feedback saved. This is not proof of model consumption."); setRefresh(n => n + 1); } } catch (e) { report(e); } finally { if (!lifecycle.current.signal.aborted) setSaving(false); }
  }
  const recordViews = ["timeline", "memory", "decisions", "graph"].includes(view);
  const closeDetail = () => { selection.current++; setSelected(null); setAudit(null); setFeedback(null); };
  return <div className="view local-workspace">
    <div className="view__head"><div><h1 className="view__title">{titles[view]}</h1><p className="view__subtitle">{repository ? <>Local records in <span className="mono">{repository}</span></> : "Choose an exact repository to inspect its local records."}</p></div><div className="view__actions"><button className="btn" onClick={() => setRefresh(n => n + 1)} disabled={!repository}>Refresh</button>{recordViews && <button className="btn btn--primary" disabled={!repository} onClick={() => setEditor({ original: null })}><Icon name="memory" size={13} /> New memory</button>}</div></div>
    {error && <div className="local-error" role="alert">{error}</div>}{notice && <p className="local-notice" role="status">{notice}</p>}
    {view === "timeline" && inventory && <Inventory value={Object.fromEntries(["records", "memories", "events", "links"].filter(key => typeof inventory[key] === "number").map(key => [key, inventory[key]]))} />}
    {recordViews && <div className="filter-bar local-toolbar"><input className="local-search" aria-label="Search memories" placeholder="Search memories..." value={searchText} onChange={e => { resetPage(); onSearch(e.target.value); }} />{view !== "decisions" && <select aria-label="Kind" value={kind} onChange={e => { resetPage(); setKind(e.target.value); }}><option value="">All kinds</option>{["event", "decision", "lesson", "preference", "note"].map(k => <option key={k}>{k}</option>)}</select>}<label><input type="checkbox" checked={inactive} onChange={e => { resetPage(); setInactive(e.target.checked); }} /> Include inactive</label><label><input type="checkbox" checked={ranking} onChange={e => setRanking(e.target.checked)} /> Feedback ranking</label></div>}
    {view === "audit" && <div className="filter-bar local-toolbar"><select aria-label="Audit kind" value={kind} onChange={e => { resetPage(); setKind(e.target.value); }}><option value="">All evidence</option><option value="delivery">Delivery</option><option value="observation">Observation</option></select><label><input type="checkbox" checked={archived} onChange={e => { resetPage(); setArchived(e.target.checked); }} /> Include archived</label></div>}
    {loading ? <p className="local-empty">Loading local records...</p> : !repository ? <p className="local-empty">No repository selected.</p> : <>
      {view === "graph" && <section><p className="view__subtitle">Select a record to explore its explicit local connections. No inferred relationships are shown.</p>{graph && <LocalGraph graph={graph} onSelect={id => void openRecord(id)} />}</section>}
      {recordViews && (rows.length ? view === "timeline" ? <div className="timeline">{rows.map(r => <button type="button" className="timeline__row" key={r.id} onClick={() => void openRecord(r.id)}><time className="timeline__time" dateTime={r.updated_at} title={r.updated_at}><span>{r.updated_at.slice(11, 19) || "--:--"}</span><span className="local-timeline-date">{r.updated_at.slice(0, 10)}</span></time><span className="timeline__type" data-kind={r.kind} title={r.kind} aria-label={r.kind}><Icon name={r.kind === "event" ? "timeline" : r.kind === "decision" ? "decision" : "memory"} size={13} /></span><span className="timeline__main"><span className="timeline__title">{r.title || r.id}</span><span className="timeline__detail">{r.body}</span></span><span className="timeline__meta"><span>{r.repository}</span><Tag>{r.authority}</Tag></span></button>)}</div> : view === "decisions" ? <div className="decision-list">{rows.map(r => <button type="button" key={r.id} className={`decision ${r.authority === "superseded" ? "is-superseded" : ""}`} onClick={() => void openRecord(r.id)}><div className="decision__head"><span className="decision__id">{r.id}</span><span className="decision__title">{r.title}</span><Tag>{r.authority}</Tag></div><p className="decision__rationale">{r.body}</p><div className="decision__foot"><Tag>{r.repository}</Tag><span className="mono">revision {r.revision}</span></div></button>)}</div> : <div className="memory-grid">{rows.map(r => <button type="button" className="memory" key={r.id} onClick={() => void openRecord(r.id)}><div className="memory__head"><span className="memory__kind">{r.kind}</span><span className="mono">v{r.revision}</span></div><div className="memory__title">{r.title || "Untitled record"}</div><div className="memory__excerpt">{r.body}</div><div className="memory__foot"><Tag tone="solid">{r.authority}</Tag><Tag>{r.repository}</Tag>{explanations[r.id] ? <Tag>Feedback explanation available</Tag> : null}</div></button>)}</div> : <p className="local-empty">No records match this repository and filter.</p>)}
      {view === "health" && <><p className="view__subtitle">Local file/index inventory. Counts do not establish productivity or model use.</p><Inventory value={summary} /></>}
      {view === "classifications" && <><p className="view__subtitle">Local queue only. Open a memory to enqueue it; provider processing requires the CLI and explicit authorization.</p>{jobs.length ? <div className="local-scroll"><table className="local-table"><thead><tr><th>Record</th><th>Status</th><th>Attempts</th><th>Known tokens</th></tr></thead><tbody>{jobs.map((j, i) => <tr key={`${j.prepared.record_id}-${i}`}><td><button className="btn btn--ghost" onClick={() => void openRecord(j.prepared.record_id)}>{j.prepared.record_id} / v{j.prepared.revision}</button></td><td>{j.status}</td><td>{j.attempts}</td><td>{j.known_input_tokens + j.known_output_tokens}{j.unknown_usage_attempts ? " (usage partly unknown)" : ""}</td></tr>)}</tbody></table></div> : <p className="local-empty">No queued classifications in this repository.</p>}<Json value={summary} /></>}
      {view === "audit" && <><details><summary>Evidence metrics and sample boundaries</summary><p>Prepared deliveries and captured citations are not proof of causal model use.</p><Json value={summary} /></details><div className="local-scroll"><table className="local-table"><thead><tr><th>Evidence</th><th>Recorded</th><th>State</th></tr></thead><tbody>{auditRows.map(e => <tr key={e.record.id}><td><button className="btn btn--ghost" onClick={() => void openAudit(e.record.id)}>{e.record.evidence_label.replaceAll("_", " ")}</button></td><td><time className="local-table-time" dateTime={e.record.recorded_at} title={e.record.recorded_at}><span>{e.record.recorded_at.slice(11, 19) || "--:--"}</span><span>{e.record.recorded_at.slice(0, 10)}</span></time></td><td>{e.archived ? "Archived" : "Current"}</td></tr>)}</tbody></table></div>{!auditRows.length && <p className="local-empty">No audit evidence in this view.</p>}</>}
      {view === "feedback" && <><p className="view__subtitle">Declared judgments. Choose an exact delivery in Delivery audit to record feedback.</p><button className="btn" onClick={() => onNavigate("audit")}>Open delivery audit</button><div className="local-scroll"><table className="local-table"><thead><tr><th>Judgment</th><th>Eligibility</th><th>Recorded</th></tr></thead><tbody>{feedbackRows.map(e => <tr key={e.record.request.id}><td><button className="btn btn--ghost" onClick={() => void openFeedback(e.record.request.id)}>{e.record.request.judgment}</button></td><td>{e.eligibility}</td><td>{e.record.recorded_at}</td></tr>)}</tbody></table></div>{!feedbackRows.length && <p className="local-empty">No feedback in this exact repository. Legacy imports remain in legacy-feedback-unattributed.</p>}</>}
    </>}
    {(recordViews || view === "audit" || view === "feedback") && <div className="local-row"><button className="btn" disabled={loading || cursors.length === 1} onClick={() => setCursors(c => c.slice(0, -1))}>Previous</button><span>Page {cursors.length}{recordViews && searchText.trim() ? " / up to 100 search matches" : ""}</span><button className="btn" disabled={loading || !next} onClick={() => { if (next) setCursors(c => [...c, next]); }}>Next</button></div>}
    <DetailDrawer open={!!selected || !!audit || !!feedback} onClose={closeDetail} title={selected?.title || audit?.evidence_label || "Feedback detail"} subtitle={repository}>
      {selected && <><div className="local-row"><Tag>{selected.kind}</Tag><Tag>{selected.authority}</Tag><Tag>v{selected.revision}</Tag><button className="btn" disabled={selected.kind === "event"} onClick={() => { setEditor({ original: selected }); closeDetail(); }}>Revise memory</button><button className="btn" onClick={() => { void request("enqueue", { id: selected.id }).then(() => { if (!lifecycle.current.signal.aborted) setNotice("Classification queued locally. No provider request sent."); }).catch(report); }}>Queue classification</button></div><div className="local-body">{selected.body}</div><h3>Source and integrity</h3><Json value={{ id: selected.id, source: selected.source, metadata: selected.metadata, content_sha256: selected.content_sha256 }} />{explanations[selected.id] ? <><h3>Declared feedback explanation</h3><Json value={explanations[selected.id]} /></> : null}<h3>Revision history</h3><div className="local-row"><input aria-label="Revision" type="number" min={1} max={selected.revision} value={revision} onChange={e => setRevision(Number(e.target.value))} /><button className="btn" onClick={() => { const generation = selection.current; void request<LocalRecord>("get", { id: selected.id, revision }).then(r => { if (!lifecycle.current.signal.aborted && generation === selection.current) setHistorical(r); }).catch(report); }}>Read revision</button></div>{historical && <Json value={historical} />}<h3>Connections</h3><button className="btn" onClick={() => void loadGraph(selected)}>Explore connections</button>{graph && <LocalGraph graph={graph} onSelect={id => void openRecord(id)} />}</>}
      {audit && <><Json value={audit} />{audit.payload.content.delivery_id && <button className="btn" onClick={() => void openAudit(audit.payload.content.delivery_id!)}>Inspect linked delivery</button>}{audit.payload.type === "delivery" && <button className="btn" onClick={() => judge(audit)}>Judge delivered bundle</button>}{[...(audit.payload.content.event ? [audit.payload.content.event] : []), ...(audit.payload.content.references ?? []), ...(audit.payload.content.evidence ?? [])].map((ref, i) => <div className="local-reference" key={`${ref.id}-${i}`}><Json value={ref} /><div className="local-row"><button className="btn" onClick={() => void openRecord(ref.id, ref)}>Read exact source revision</button>{audit.payload.type === "delivery" && <button className="btn" onClick={() => judge(audit, ref)}>Judge exact delivered record</button>}</div></div>)}</>}
      {feedback && <Json value={feedback} />}
    </DetailDrawer>
    <DetailDrawer open={!!editor} onClose={() => { if (!saving) setEditor(null); }} title={editor?.original ? "Revise memory" : "Capture memory"} subtitle={repository}>{editor && <MemoryEditor key={editor.original?.id ?? "new"} original={editor.original} saving={saving} error={error} onSave={async payload => { setSaving(true); setError(""); try { const original = editor.original; const result = await request<{ record: LocalRecord }>(original ? "revise" : "capture", original ? { id: original.id, expected_revision: original.revision, request: payload } : payload); if (!lifecycle.current.signal.aborted) { setEditor(null); setRefresh(n => n + 1); selection.current++; setSelected(result.record); setHistorical(null); setGraph(null); setRevision(result.record.revision); setAudit(null); setFeedback(null); setNotice(`Memory saved as revision ${result.record.revision}.`); } } catch (e) { report(e); } finally { if (!lifecycle.current.signal.aborted) setSaving(false); } }} />}</DetailDrawer>
    <DetailDrawer open={!!target} onClose={() => { if (!saving) { setTarget(null); frozenFeedback.current = null; } }} title="Record declared feedback" subtitle={repository}>{target && <form className="local-form" onSubmit={e => void saveFeedback(e)}><p>{target.reference ? `${target.reference.id} / revision ${target.reference.revision}` : "Bundle judgment is not record-specific ranking evidence."}</p><details><summary>Delivery reference</summary><Json value={{ id: target.id, delivery_id: target.delivery_id, reference: target.reference }} /></details><label>Judgment<select disabled={saving || !!frozenFeedback.current} value={target.judgment} onChange={e => setTarget({ ...target, judgment: e.target.value })}><option value="unknown">Unknown</option><option value="helpful">Helpful</option><option value="unhelpful">Unhelpful</option></select></label><label>Declared actor<input required disabled={saving || !!frozenFeedback.current} value={target.source.actor} onChange={e => setTarget({ ...target, source: { ...target.source, actor: e.target.value } })} /></label><label>Source reference<input required disabled={saving || !!frozenFeedback.current} value={target.source.reference} onChange={e => setTarget({ ...target, source: { ...target.source, reference: e.target.value } })} /></label><label>Comment<textarea maxLength={4096} disabled={saving || !!frozenFeedback.current} value={target.comment} onChange={e => setTarget({ ...target, comment: e.target.value })} /></label>{error && <div role="alert" className="local-error">{error}</div>}<button className="btn btn--primary" disabled={saving}>{saving ? "Saving..." : frozenFeedback.current ? "Retry exact feedback" : "Save feedback"}</button></form>}</DetailDrawer>
  </div>;
}
function Json({ value }: { value: unknown }) { return <pre className="local-pre">{pretty(value)}</pre>; }

function MemoryEditor({ original, saving, error, onSave }: { original: LocalRecord | null; saving: boolean; error: string; onSave: (payload: object) => Promise<void> }) {
  const [title, setTitle] = useState(original?.title ?? ""); const [body, setBody] = useState(original?.body ?? "");
  const [kind, setKind] = useState(original?.kind ?? "note"); const [authority, setAuthority] = useState(original?.authority ?? "observation");
  const [links, setLinks] = useState(original?.links ?? []);
  return <form className="local-form" onSubmit={e => { e.preventDefault(); void onSave({ title, body, authority, links, metadata: original?.metadata ?? {}, ...(!original ? { kind } : {}) }); }}><p>{original ? `Creates revision ${original.revision + 1}; prior evidence remains intact.` : "Capture a sourced local memory."}</p><label>Title<input required value={title} onChange={e => setTitle(e.target.value)} /></label><label>Body<textarea required value={body} onChange={e => setBody(e.target.value)} /></label><label>Kind<select disabled={!!original} value={kind} onChange={e => setKind(e.target.value)}>{["note", "decision", "lesson", "preference"].map(k => <option key={k}>{k}</option>)}</select></label><label>Authority<select value={authority} onChange={e => setAuthority(e.target.value)}>{["observation", "proposed", "accepted", "deprecated", "superseded"].map(a => <option key={a}>{a}</option>)}</select></label><h3>Connections</h3>{links.map((link, i) => <div className="local-row" key={i}><input required aria-label={`Target memory ${i + 1}`} value={link.target_id} onChange={e => setLinks(links.map((l, n) => n === i ? { ...l, target_id: e.target.value } : l))} /><select aria-label={`Relation ${i + 1}`} value={link.relation} onChange={e => setLinks(links.map((l, n) => n === i ? { ...l, relation: e.target.value } : l))}>{["related_to", "supports", "contradicts", "derived_from", "supersedes"].map(r => <option key={r}>{r}</option>)}</select><button type="button" className="btn" onClick={() => setLinks(links.filter((_, n) => n !== i))}>Remove</button></div>)}<button type="button" className="btn" onClick={() => setLinks([...links, { target_id: "", relation: "related_to" }])}>Add connection</button>{error && <div className="local-error" role="alert">{error}</div>}<button className="btn btn--primary" disabled={saving}>{saving ? "Saving..." : "Save memory"}</button></form>;
}
function LocalGraph({ graph, onSelect }: { graph: GraphData; onSelect: (id: string) => void }) {
  const positions = new Map(graph.records.map((r, i) => [r.id, { x: 300 + Math.cos(i * Math.PI * 2 / Math.max(graph.records.length, 1)) * 200, y: 180 + Math.sin(i * Math.PI * 2 / Math.max(graph.records.length, 1)) * 120 }]));
  return <><p>{graph.records.length} records / {graph.edges.length} explicit connections{graph.truncated ? " / bounded neighborhood" : ""}</p><svg className="local-graph" viewBox="0 0 600 360" aria-label="Explicit memory connections">{graph.edges.map((e, i) => { const a = positions.get(e.source_id), b = positions.get(e.target_id); return a && b ? <line key={i} x1={a.x} y1={a.y} x2={b.x} y2={b.y} /> : null; })}{graph.records.map(r => { const p = positions.get(r.id)!; return <g key={r.id} role="button" tabIndex={0} aria-label={r.title} onClick={() => onSelect(r.id)} onKeyDown={e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onSelect(r.id); } }}><title>{r.title}</title><circle cx={p.x} cy={p.y} r={10} /><text x={p.x} y={p.y + 26} textAnchor="middle">{r.title.slice(0, 24)}</text></g>; })}</svg>{graph.edges.length ? <ul>{graph.edges.map((e, i) => <li key={i}><button className="btn btn--ghost" onClick={() => onSelect(e.source_id)}>{graph.records.find(r => r.id === e.source_id)?.title ?? e.source_id}</button> {e.relation} <button className="btn btn--ghost" onClick={() => onSelect(e.target_id)}>{graph.records.find(r => r.id === e.target_id)?.title ?? e.target_id}</button></li>)}</ul> : <p>No explicit connections. Revise a memory to add one.</p>}</>;
}

function Inventory({ value }: { value: unknown }) {
  if (!value || typeof value !== "object") return null;
  return <div className="stats-grid">{Object.entries(value).filter(([, count]) => typeof count === "number").map(([label, count]) => <div className="stat" key={label}><div className="stat__label">{label}</div><div className="stat__value tabular">{count.toLocaleString()}</div></div>)}</div>;
}
