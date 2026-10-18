"use strict";
const $ = (id) => document.getElementById(id);
const state = { repository: "", records: [], selected: null, editing: null, generation: 0, selection: 0 };
const auditState = { generation: 0, selection: 0, source: 0, cursors: [null], page: 0, next: null };
const feedbackState = { generation: 0, selection: 0, cursors: [null], page: 0, next: null, target: null };
const searchFeedback = new Map();
const parameters = new URLSearchParams(location.hash.slice(1));
const bootstrap = parameters.get("token");
state.repository = parameters.get("repository") || "";
history.replaceState(null, "", location.pathname);
let token = bootstrap || "";
try {
  if (bootstrap && /^[a-f0-9]{64}$/i.test(bootstrap)) sessionStorage.setItem("cortex-session", bootstrap);
  token = sessionStorage.getItem("cortex-session") || token;
  state.repository = state.repository || sessionStorage.getItem("cortex-repository") || "";
  if (state.repository) sessionStorage.setItem("cortex-repository", state.repository);
} catch (_) { /* Private storage may be unavailable; the current tab still works. */ }
$("repository").value = state.repository;

function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function notice(message, error = false) {
  $("notice").textContent = message;
  $("notice").className = error ? "error" : "";
  $("notice").hidden = !message;
}
async function api(endpoint, data) {
  if (!token) throw new Error("Open the full startup URL printed by cortex serve to connect this tab.");
  if (!data.repository) throw new Error("Enter the exact repository ID to open its memories.");
  const response = await fetch(`/api/${endpoint}`, {
    method: "POST", credentials: "omit", cache: "no-store",
    headers: { "Content-Type": "application/json", "Authorization": `Bearer ${token}` },
    body: JSON.stringify(data)
  });
  if (!response.ok) {
    let message = `Request failed (${response.status}).`;
    if ((response.headers.get("content-type") || "").includes("application/json")) {
      const error = await response.json();
      message = error.error || message;
    } else if (response.status === 413) message = "Memory exceeds the web request size limit.";
    if (response.status === 401) $("connection").textContent = "Reconnect required";
    throw new Error(message);
  }
  $("connection").textContent = "Connected locally";
  return response.json();
}
function run(action) { return async (event) => { if (event) event.preventDefault(); try { await action(event); } catch (error) { notice(error.message, true); } }; }
function displayDate(value) { const date = new Date(value); return Number.isNaN(date.getTime()) ? value : date.toLocaleString(); }
function pretty(value) { return JSON.stringify(value, null, 2); }

async function refresh() {
  const generation = ++state.generation;
  const repository = state.repository;
  const query = $("search").value.trim();
  const data = { repository, limit: 100, include_inactive: $("include-inactive").checked };
  const searchRequest = { ...data, text: query };
  if ($("feedback-ranking").checked) searchRequest.feedback = true;
  const hits = query ? await api("search", searchRequest) : null;
  const rows = hits ? hits.map((hit) => hit.record) : await api("recent", data);
  if (generation !== state.generation) return;
  searchFeedback.clear();
  for (const hit of hits || []) { if (hit.feedback) searchFeedback.set(hit.record.id, hit.feedback); }
  renderSearchFeedback();
  state.records = rows;
  renderResults();
  $("library-summary").textContent = `${rows.length === 100 ? "Showing up to " : ""}${rows.length} ${query ? "matches" : "recent memories"}`;
  const counts = await api("status", { repository });
  if (generation !== state.generation) return;
  $("scope-summary").textContent = `${counts.memories} memories / ${counts.events} events / ${counts.revisions} revisions`;
  await refreshQueue();
  if (generation === state.generation) await refreshAudit(true);
  if (generation === state.generation) await refreshFeedback(true);
}
function resetFeedback() {
  feedbackState.generation += 1; feedbackState.selection += 1;
  feedbackState.cursors = [null]; feedbackState.page = 0; feedbackState.next = null; feedbackState.target = null;
  $("feedback-list").replaceChildren(); $("feedback-detail").textContent = "Select feedback to inspect its provenance and eligibility.";
  $("feedback-form").reset(); $("feedback-form").hidden = true; $("feedback-error").hidden = true;
  $("feedback-page").textContent = ""; $("feedback-back").disabled = true; $("feedback-next").disabled = true;
}
async function refreshFeedback(reset = false) {
  if (reset) resetFeedback();
  const generation = ++feedbackState.generation, repository = state.repository;
  const current = () => repository === state.repository && generation === feedbackState.generation;
  $("feedback-back").disabled = true; $("feedback-next").disabled = true;
  try {
    const page = await api("feedback/list", { repository, limit: 20, cursor: feedbackState.cursors[feedbackState.page] });
    if (!current()) return;
    const list = $("feedback-list"); list.replaceChildren();
    for (const entry of page.entries) {
      const request = entry.record.request, button = element("button", undefined, "result"); button.type = "button";
      button.append(element("span", `${request.judgment} / ${request.evidence}`, "result-title"));
      button.append(element("span", `${entry.eligibility} / ${request.repository}`, "result-context"));
      button.append(element("span", request.id, "record-id"));
      button.addEventListener("click", () => selectFeedback(request.id)); list.append(button);
    }
    if (!page.entries.length) list.append(element("p", "No feedback in this exact repository. Imported legacy rows are visible only in legacy-feedback-unattributed.", "muted"));
    feedbackState.next = page.next_cursor;
    $("feedback-page").textContent = `Page ${feedbackState.page + 1} / ${page.entries.length} records`;
    $("feedback-back").disabled = feedbackState.page === 0; $("feedback-next").disabled = !page.next_cursor;
  } catch (error) { if (current()) { $("feedback-error").textContent = error.message; $("feedback-error").hidden = false; } }
}
async function selectFeedback(id) {
  const repository = state.repository, selection = ++feedbackState.selection;
  try {
    const entry = await api("feedback/get", { repository, id });
    if (repository !== state.repository || selection !== feedbackState.selection) return;
    $("feedback-detail").textContent = pretty(entry);
  } catch (error) { if (repository === state.repository && selection === feedbackState.selection) { $("feedback-error").textContent = error.message; $("feedback-error").hidden = false; } }
}
function feedbackButton(delivery, reference) {
  const button = element("button", reference ? "Judge exact delivered record" : "Judge delivered bundle"); button.type = "button";
  button.addEventListener("click", () => {
    if (delivery.repository !== state.repository) return;
    feedbackState.target = { repository: delivery.repository, query_id: delivery.payload.content.query_id, delivery_id: delivery.id, reference };
    $("feedback-form").reset(); $("feedback-form").hidden = false; $("feedback-id").value = `feedback-${crypto.randomUUID()}`;
    $("feedback-target").textContent = reference ? `${reference.id} / revision ${reference.revision} / ${reference.content_sha256}` : `Bundle ${delivery.id}; bundle judgments are not record-specific ranking evidence.`;
    $("feedback-form").scrollIntoView({ block: "nearest" });
  });
  return button;
}
async function saveFeedback(event) {
  event.preventDefault();
  const target = feedbackState.target; if (!target || target.repository !== state.repository) return;
  const request = { id: $("feedback-id").value, repository: target.repository,
    query_id: target.query_id, delivery_id: target.delivery_id,
    source: { kind: $("feedback-kind").value, reference: $("feedback-source").value, actor: $("feedback-actor").value },
    evidence: "explicit", judgment: $("feedback-judgment").value };
  if (target.reference) request.reference = target.reference;
  if ($("feedback-comment").value) request.comment = $("feedback-comment").value;
  $("feedback-save").disabled = true; $("feedback-error").hidden = true;
  try {
    const result = await api("feedback/record", request);
    if (feedbackState.target !== target || target.repository !== state.repository) return;
    $("feedback-detail").textContent = pretty(result.entry); $("feedback-form").hidden = true;
    feedbackState.target = null; feedbackState.page = 0; feedbackState.cursors = [null];
    await refreshFeedback();
  } catch (error) { if (feedbackState.target === target && target.repository === state.repository) { $("feedback-error").textContent = error.message; $("feedback-error").hidden = false; } }
  finally { $("feedback-save").disabled = false; }
}
function resetAudit() {
  auditState.generation += 1; auditState.selection += 1; auditState.source += 1;
  auditState.cursors = [null]; auditState.page = 0; auditState.next = null;
  for (const id of ["audit-list", "audit-metrics", "audit-detail"]) $(id).replaceChildren();
  $("audit-source-panel").hidden = true; $("audit-source").textContent = "";
  $("audit-denominator").textContent = ""; $("audit-page").textContent = "";
  $("audit-error").hidden = true; $("audit-back").disabled = true; $("audit-next").disabled = true;
}
async function refreshAudit(reset = false) {
  if (reset) resetAudit();
  const generation = ++auditState.generation, repository = state.repository;
  const current = () => generation === auditState.generation && repository === state.repository;
  const filters = { repository, include_archived: $("audit-archived").checked };
  const query = { ...filters, limit: 20, cursor: auditState.cursors[auditState.page] };
  if ($("audit-kind").value) query.kind = $("audit-kind").value;
  $("audit-next").disabled = true; $("audit-back").disabled = true;
  $("audit-error").hidden = true;
  try {
    const [page, metrics] = await Promise.all([api("audit/list", query), api("audit/metrics", { ...filters, limit: 10000 })]);
    if (!current()) return;
    const list = $("audit-list"); list.replaceChildren();
    for (const entry of page.entries) {
      const record = entry.record, button = element("button", undefined, "result"); button.type = "button";
      button.append(element("span", record.evidence_label.replaceAll("_", " "), "result-title"));
      button.append(element("span", `${record.recorded_at} / ${record.payload.type}${entry.archived ? " / archived" : ""}`, "result-context"));
      button.append(element("span", record.id, "record-id"));
      button.addEventListener("click", () => selectAudit(record.id)); list.append(button);
    }
    if (!page.entries.length) list.append(element("p", "No audit evidence in this view.", "muted"));
    const cards = $("audit-metrics"); cards.replaceChildren();
    for (const [label, value] of [["Prepared deliveries", metrics.deliveries.prepared], ["Related actions", metrics.observations.related], ["Exact citation evidence", metrics.observations.captured_citations], ["Unknown deliveries / actions", `${metrics.deliveries.unknown} / ${metrics.observations.unknown}`], ["Preparation errors", metrics.deliveries.errors], ["Empty deliveries", metrics.deliveries.empty]]) {
      const card = element("div"); card.append(element("strong", String(value)), element("span", label)); cards.append(card);
    }
    $("audit-denominator").textContent = `All kinds in this repository and archive filter: ${metrics.sampled_records} sampled / ${metrics.matching_records} matching / ${metrics.omitted_records} omitted; truncated: ${metrics.truncated}. Delivery attempts: ${metrics.deliveries.attempts}; actions: ${metrics.observations.total}. Captured citations, not causal use: ${metrics.prepared_deliveries_with_citation} prepared deliveries with citations / ${metrics.deliveries.prepared} prepared deliveries; rate ${metrics.citation_delivery_rate === null ? "unavailable" : (metrics.citation_delivery_rate * 100).toFixed(1) + "%"}. Citation observations with delivery outside sample: ${metrics.citation_observations_with_delivery_outside_sample}.`;
    auditState.next = page.next_cursor;
    $("audit-page").textContent = `Page ${auditState.page + 1} / ${page.entries.length} records`;
    $("audit-back").disabled = auditState.page === 0; $("audit-next").disabled = !page.next_cursor;
  } catch (error) { if (current()) { $("audit-error").textContent = error.message; $("audit-error").hidden = false; } }
}
async function selectAudit(id) {
  const repository = state.repository, selection = ++auditState.selection;
  auditState.source += 1; $("audit-source-panel").hidden = true; $("audit-source").textContent = "";
  const current = () => repository === state.repository && selection === auditState.selection;
  const detail = $("audit-detail"); detail.replaceChildren(element("p", "Loading evidence...", "muted"));
  try {
    const record = await api("audit/get", { repository, id });
    if (!current()) return;
    if (!record || record.repository !== repository) throw new Error("Audit record unavailable in this repository.");
    const payload = record.payload.content;
    detail.replaceChildren(element("h3", record.evidence_label.replaceAll("_", " ")));
    detail.append(element("p", `${record.recorded_at} / ${record.payload.type}`, "muted"));
    const summary = { id: record.id, repository, content_sha256: record.content_sha256, verification: record.verification, ...payload };
    detail.append(element("pre", pretty(summary)));
    if (record.payload.type === "delivery") detail.append(feedbackButton(record, null));
    if (payload.delivery_id) {
      const button = element("button", "Inspect linked delivery"); button.type = "button";
      button.addEventListener("click", () => selectAudit(payload.delivery_id)); detail.append(button);
    }
    const references = [...(payload.references || []), ...(payload.evidence || [])];
    if (payload.event) references.unshift(payload.event);
    for (const reference of references) {
      const block = element("div", undefined, "audit-reference");
      block.append(element("h3", `${reference.id} / revision ${reference.revision}`), element("pre", pretty(reference)));
      const button = element("button", payload.event === reference ? "Read exact action revision" : "Read exact source revision"); button.type = "button";
      button.addEventListener("click", () => readAuditSource(reference, selection)); block.append(button); detail.append(block);
      if (record.payload.type === "delivery") block.append(feedbackButton(record, reference));
    }
  } catch (error) { if (current()) detail.replaceChildren(element("p", error.message, "error")); }
}
async function readAuditSource(reference, selection) {
  const repository = state.repository, source = ++auditState.source;
  const current = () => repository === state.repository && selection === auditState.selection && source === auditState.source;
  $("audit-source-panel").hidden = false; $("audit-source").textContent = "Loading exact historical revision...";
  try {
    if (reference.repository !== repository) throw new Error("Source scope differs from current repository.");
    const record = await api("get", { repository, id: reference.id, revision: reference.revision });
    if (!current()) return;
    if (!record || record.repository !== repository || record.id !== reference.id || record.revision !== reference.revision || record.content_sha256 !== reference.content_sha256) throw new Error("Historical source does not match audit reference.");
    $("audit-source").textContent = pretty(record);
  } catch (error) { if (current()) $("audit-source").textContent = error.message; }
}
function renderResults() {
  const container = $("results"); container.replaceChildren();
  if (!state.records.length) {
    container.append(element("p", "No memories found. Capture a memory, or try another search.", "muted"));
    return;
  }
  for (const record of state.records) {
    const button = element("button", undefined, "result"); button.type = "button";
    button.setAttribute("aria-current", String(state.selected?.id === record.id));
    button.append(element("span", record.title || "Untitled memory", "result-title"));
    button.append(element("span", `${record.kind} / ${record.authority} / v${record.revision}`, "result-context"));
    button.append(element("span", record.body, "result-excerpt"));
    const explanation = searchFeedback.get(record.id);
    if (explanation) button.append(element("span", `${explanation.reason}: ${explanation.baseline_score.toFixed(4)} + ${explanation.adjustment.toFixed(4)} = ${explanation.final_score.toFixed(4)}; ${explanation.samples} declared query groups`, "result-context"));
    button.addEventListener("click", run(() => select(record.id)));
    container.append(button);
  }
}
function renderSearchFeedback() {
  const explanation = state.selected ? searchFeedback.get(state.selected.id) : null;
  $("search-feedback").hidden = !explanation;
  $("search-feedback").textContent = explanation ? `Declared feedback search explanation (not independent votes or consumption)\n${pretty(explanation)}` : "";
}
async function select(id) {
  const selection = ++state.selection;
  const repository = state.repository;
  const record = await api("get", { repository, id });
  if (repository !== state.repository || selection !== state.selection) return;
  state.selected = record;
  renderSearchFeedback();
  $("empty").hidden = true; $("detail").hidden = false;
  $("record-title").textContent = record.title;
  $("record-body").textContent = record.body;
  $("record-kind").textContent = record.kind;
  $("record-authority").textContent = record.authority;
  $("record-revision").textContent = `Revision ${record.revision}`;
  $("record-id").textContent = record.id;
  $("record-source").textContent = pretty(record.source);
  $("record-metadata").textContent = pretty(record.metadata || {});
  $("edit-memory").disabled = record.kind === "event";
  $("edit-memory").title = record.kind === "event" ? "Events are immutable; capture a derived memory instead." : "Create a new revision";
  const integrity = $("record-integrity"); integrity.replaceChildren();
  for (const [label, value] of [["Created", displayDate(record.created_at)], ["Updated", displayDate(record.updated_at)], ["Content SHA-256", record.content_sha256], ["Repository", record.repository]]) {
    integrity.append(element("dt", label), element("dd", value));
  }
  $("history-revision").value = record.revision; $("history-revision").max = record.revision;
  $("graph").replaceChildren(); $("graph-links").replaceChildren(); $("graph-summary").textContent = "";
  renderHistory(record); showTab("content"); renderResults();
}
function showTab(name) {
  document.querySelectorAll("[data-tab]").forEach((button) => {
    const active = button.dataset.tab === name;
    button.setAttribute("aria-pressed", String(active));
    $(`panel-${button.dataset.tab}`).hidden = !active;
  });
}
function renderHistory(record) {
  $("history-meta").textContent = `${record.authority} / ${displayDate(record.updated_at)} / revision ${record.revision}`;
  $("history-title").textContent = record.title; $("history-body").textContent = record.body;
  $("history-evidence").textContent = pretty({ source: record.source, links: record.links, metadata: record.metadata, content_sha256: record.content_sha256 });
}
async function loadHistory() {
  if (!state.selected) return;
  const selected = state.selected;
  const revision = Number($("history-revision").value);
  if (!Number.isSafeInteger(revision) || revision < 1 || revision > selected.revision) throw new Error("Choose an existing revision number.");
  const record = await api("get", { repository: selected.repository, id: selected.id, revision });
  if (state.selected === selected) renderHistory(record);
}
function svgElement(tag, attributes, text) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  Object.entries(attributes).forEach(([key, value]) => node.setAttribute(key, String(value)));
  if (text !== undefined) node.textContent = text;
  return node;
}
async function loadGraph() {
  const selected = state.selected; if (!selected) return;
  const graph = await api("graph", { repository: selected.repository, id: selected.id, depth: 2, limit: 30 });
  if (state.selected !== selected) return;
  const svg = $("graph"); svg.replaceChildren();
  const positions = new Map();
  const others = graph.records.filter((record) => record.id !== selected.id);
  positions.set(selected.id, { x: 340, y: 210 });
  others.forEach((record, index) => {
    const angle = (index / Math.max(others.length, 1)) * Math.PI * 2 - Math.PI / 2;
    positions.set(record.id, { x: 340 + Math.cos(angle) * 240, y: 210 + Math.sin(angle) * 145 });
  });
  for (const edge of graph.edges) {
    const source = positions.get(edge.source_id), target = positions.get(edge.target_id);
    if (source && target) svg.append(svgElement("line", { x1: source.x, y1: source.y, x2: target.x, y2: target.y, class: "graph-edge" }));
  }
  for (const record of graph.records) {
    const position = positions.get(record.id); if (!position) continue;
    const group = svgElement("g", { class: `graph-node${record.id === selected.id ? " root" : ""}`, tabindex: "0", role: "button", "aria-label": record.title });
    group.append(svgElement("title", {}, record.title));
    group.append(svgElement("circle", { cx: position.x, cy: position.y, r: record.id === selected.id ? 15 : 9 }));
    group.append(svgElement("text", { x: position.x, y: position.y + 31, "text-anchor": "middle" }, record.title.length > 24 ? `${record.title.slice(0, 21)}...` : record.title));
    group.addEventListener("click", run(() => select(record.id)));
    group.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); run(() => select(record.id))(); } });
    svg.append(group);
  }
  $("graph-summary").textContent = `${graph.records.length} memories / ${graph.edges.length} connections${graph.truncated ? " / showing a bounded neighborhood" : ""}`;
  const list = $("graph-links"); list.replaceChildren();
  for (const edge of graph.edges) {
    const source = graph.records.find((record) => record.id === edge.source_id)?.title || edge.source_id;
    const target = graph.records.find((record) => record.id === edge.target_id)?.title || edge.target_id;
    list.append(element("li", `${source} / ${edge.relation.replaceAll("_", " ")} / ${target}`));
  }
  if (!graph.edges.length) list.append(element("li", "No connections yet. Revise this memory to add a connection."));
}
async function refreshQueue() {
  const repository = state.repository;
  const result = await api("queue", { repository, limit: 100 });
  if (repository !== state.repository) return;
  const list = $("queue-list"); list.replaceChildren();
  for (const job of result.jobs) {
    const row = element("div", undefined, "queue-item");
    const title = state.records.find((record) => record.id === job.prepared.record_id)?.title || job.prepared.record_id;
    row.append(element("span", `${title} / revision ${job.prepared.revision}`));
    row.append(element("span", `${job.status} / ${job.attempts} attempts / ${job.known_input_tokens + job.known_output_tokens} known tokens${job.unknown_usage_attempts ? " / usage partly unknown" : ""}`, "queue-state"));
    list.append(row);
  }
  if (!result.jobs.length) list.append(element("p", "No queued classifications in this repository.", "muted"));
  if (result.malformed || result.incomplete) list.append(element("p", "Queue view is incomplete or contains invalid jobs. Inspect the queue with the CLI.", "muted"));
}
function addLink(link = { target_id: "", relation: "related_to" }) {
  const row = element("div", undefined, "link-editor");
  const input = element("input"); input.value = link.target_id; input.placeholder = "Target memory ID"; input.required = true; input.setAttribute("aria-label", "Target memory ID");
  const relation = element("select"); relation.setAttribute("aria-label", "Connection type");
  for (const value of ["related_to", "supports", "contradicts", "derived_from", "supersedes"]) { const option = element("option", value.replaceAll("_", " ")); option.value = value; relation.append(option); }
  relation.value = link.relation;
  const remove = element("button", "Remove", "quiet"); remove.type = "button"; remove.addEventListener("click", () => row.remove());
  row.append(input, relation, remove); $("editor-links").append(row);
}
function openEditor(record) {
  if (!state.repository) { notice("Open a repository before capturing a memory.", true); $("repository").focus(); return; }
  state.editing = record || null;
  $("editor-title").textContent = record ? "Revise memory" : "Capture memory";
  $("title").value = record?.title || ""; $("body").value = record?.body || "";
  $("kind").value = record?.kind || "note"; $("kind").disabled = Boolean(record);
  $("authority").value = record?.authority || "observation";
  $("editor-context").textContent = record ? `Creates revision ${record.revision + 1}; prior evidence stays intact.` : `Save in ${state.repository}.`;
  $("editor-links").replaceChildren(); (record?.links || []).forEach(addLink);
  $("editor-error").hidden = true; $("editor").showModal(); $("title").focus();
}
async function saveMemory(event) {
  event.preventDefault(); $("save-memory").disabled = true; $("editor-error").hidden = true;
  try {
    const original = state.editing;
    const links = Array.from($("editor-links").children).map((row) => ({ target_id: row.querySelector("input").value, relation: row.querySelector("select").value }));
    const request = { title: $("title").value, body: $("body").value, authority: $("authority").value, links, metadata: original?.metadata || {} };
    const result = original ? await api("revise", { repository: original.repository, id: original.id, expected_revision: original.revision, request }) : await api("capture", { repository: state.repository, kind: $("kind").value, ...request });
    $("editor").close(); notice(`Memory saved as revision ${result.record.revision}.`);
    await refresh(); await select(result.record.id);
  } catch (error) { $("editor-error").textContent = error.message; $("editor-error").hidden = false; }
  finally { $("save-memory").disabled = false; }
}

$("scope-form").addEventListener("submit", run(async () => {
  const repository = $("repository").value.trim(); if (!repository) throw new Error("Enter a repository name.");
  state.repository = repository; state.selected = null; state.records = []; $("detail").hidden = true; $("empty").hidden = false;
  try { sessionStorage.setItem("cortex-repository", repository); } catch (_) { /* Selection still works without persistence. */ }
  state.selection += 1;
  resetAudit();
  resetFeedback();
  searchFeedback.clear(); $("feedback-ranking").checked = false; renderSearchFeedback();
  $("results").replaceChildren(); $("queue-list").replaceChildren(); $("scope-summary").textContent = ""; $("library-summary").textContent = "";
  $("search").value = ""; notice(""); await refresh();
}));
$("search-form").addEventListener("submit", run(refresh));
$("refresh").addEventListener("click", run(async () => { $("search").value = ""; await refresh(); }));
$("include-inactive").addEventListener("change", run(refresh));
$("feedback-ranking").addEventListener("change", run(refresh));
$("refresh-queue").addEventListener("click", run(refreshQueue));
$("audit-refresh").addEventListener("click", () => refreshAudit(true));
$("audit-kind").addEventListener("change", () => refreshAudit(true));
$("audit-archived").addEventListener("change", () => refreshAudit(true));
$("audit-next").addEventListener("click", () => { if (auditState.next) { auditState.cursors[++auditState.page] = auditState.next; refreshAudit(); } });
$("audit-back").addEventListener("click", () => { if (auditState.page > 0) { auditState.page -= 1; refreshAudit(); } });
$("feedback-refresh").addEventListener("click", () => refreshFeedback(true));
$("feedback-next").addEventListener("click", () => { if (feedbackState.next) { feedbackState.cursors[++feedbackState.page] = feedbackState.next; refreshFeedback(); } });
$("feedback-back").addEventListener("click", () => { if (feedbackState.page > 0) { feedbackState.page -= 1; refreshFeedback(); } });
$("feedback-form").addEventListener("submit", saveFeedback);
$("new-memory").addEventListener("click", () => openEditor(null));
$("edit-memory").addEventListener("click", () => { if (state.selected) openEditor(state.selected); });
$("close-editor").addEventListener("click", () => $("editor").close());
$("editor-form").addEventListener("submit", saveMemory);
$("add-link").addEventListener("click", () => addLink());
$("load-revision").addEventListener("click", run(loadHistory));
$("enqueue").addEventListener("click", run(async () => {
  const record = state.selected; if (!record) return;
  await api("enqueue", { repository: record.repository, id: record.id });
  notice("Classification queued locally. Use the CLI to process queued work."); await refreshQueue();
}));
document.querySelectorAll("[data-tab]").forEach((button) => button.addEventListener("click", run(async () => {
  showTab(button.dataset.tab); if (button.dataset.tab === "connections") await loadGraph();
})));
if (!token) { $("connection").textContent = "Not connected"; notice("Open the full startup URL printed by cortex serve to connect this tab.", true); }
else if (state.repository) run(refresh)();
else { $("connection").textContent = "Choose repository"; notice("Enter the exact repository ID to open its memories."); $("repository").focus(); }
