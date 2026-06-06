import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Tag } from "../atoms/Tag";
import { api, type BranchMetadata } from "../lib/api";
import { useConnKey } from "../lib/connections/useConnKey";

/** Emit a cross-view event to filter the BitemporalTimeline to a branch. */
function filterTimelineBranch(project: string, branch: string): void {
  window.dispatchEvent(
    new CustomEvent("cortex:filter-timeline-branch", { detail: { project, branch } }),
  );
}

function statusTone(status: string | undefined): "ok" | "warn" | "info" | "default" {
  switch (status) {
    case "active":
      return "ok";
    case "merged":
      return "info";
    case "abandoned":
      return "warn";
    default:
      return "default";
  }
}

/** Build a parent → children map from a flat branch list. */
function buildTree(
  branches: BranchMetadata[],
): Map<string | null, BranchMetadata[]> {
  const map = new Map<string | null, BranchMetadata[]>();
  for (const b of branches) {
    const parent = b.parent_branch_id ?? null;
    const existing = map.get(parent) ?? [];
    existing.push(b);
    map.set(parent, existing);
  }
  return map;
}

export function BranchExplorerView() {
  const [project, setProject] = useState("cortex");
  const [selected, setSelected] = useState<BranchMetadata | null>(null);

  const connKey = useConnKey();
  const { data, isLoading, error } = useQuery({
    queryKey: [connKey, "branches", project],
    queryFn: () => api.branches(project),
    refetchInterval: 300_000,
    enabled: project.trim().length > 0,
  });

  const branches = data?.branches ?? [];
  const tree = buildTree(branches);

  return (
    <div className="view">
      <div className="view__head">
        <div>
          <h1 className="view__title">Branch Explorer</h1>
          <p className="view__subtitle">
            Branch tree ·{" "}
            <span className="mono">/v1/branch/{"{project}"}</span>
          </p>
        </div>
      </div>

      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 0",
          borderBottom: "1px solid var(--border)",
          marginBottom: 16,
        }}
      >
        <span style={{ color: "var(--fg-3)", fontSize: 11 }}>Project</span>
        <input
          type="text"
          value={project}
          onChange={(e) => {
            setProject(e.target.value);
            setSelected(null);
          }}
          style={{
            height: 28,
            padding: "0 10px",
            background: "var(--bg-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            color: "var(--fg-0)",
            fontSize: 11.5,
            width: 160,
          }}
          aria-label="Project name"
        />
      </div>

      {error ? (
        <Empty msg="cortex-api unreachable. Start it with cargo run -p cortex-api." />
      ) : isLoading ? (
        <Empty msg="Loading branches…" />
      ) : branches.length === 0 ? (
        <Empty msg="No branches found for this project." />
      ) : (
        <div style={{ display: "flex", gap: 24 }}>
          <div style={{ flex: 1, minWidth: 0 }}>
            <BranchTree
              tree={tree}
              parentId={null}
              depth={0}
              selected={selected}
              onSelect={setSelected}
            />
          </div>
          {selected ? (
            <BranchDetailPanel
              branch={selected}
              project={project}
              onClose={() => setSelected(null)}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}

function BranchTree({
  tree,
  parentId,
  depth,
  selected,
  onSelect,
}: {
  tree: Map<string | null, BranchMetadata[]>;
  parentId: string | null;
  depth: number;
  selected: BranchMetadata | null;
  onSelect: (b: BranchMetadata) => void;
}) {
  const children = tree.get(parentId) ?? [];
  if (children.length === 0) return null;
  return (
    <div style={{ paddingLeft: depth > 0 ? 20 : 0 }}>
      {children.map((b) => {
        const key = b.id ?? b.name ?? String(Math.random());
        const isSelected = selected?.id === b.id && selected?.name === b.name;
        return (
          <div key={key}>
            <button
              type="button"
              onClick={() => onSelect(b)}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                width: "100%",
                textAlign: "left",
                padding: "6px 10px",
                background: isSelected ? "var(--bg-3)" : "transparent",
                border: "none",
                borderRadius: "var(--radius-sm)",
                color: "var(--fg-0)",
                cursor: "pointer",
                fontSize: 12,
              }}
            >
              <span
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: 2,
                  background: "var(--accent-dim)",
                  flexShrink: 0,
                  display: "inline-block",
                }}
              />
              <span className="mono" style={{ fontSize: 12 }}>
                {b.name ?? b.id ?? "(unnamed)"}
              </span>
              <Tag tone={statusTone(b.status)}>{b.status ?? "unknown"}</Tag>
            </button>
            <BranchTree
              tree={tree}
              parentId={b.id ?? null}
              depth={depth + 1}
              selected={selected}
              onSelect={onSelect}
            />
          </div>
        );
      })}
    </div>
  );
}

function BranchDetailPanel({
  branch,
  project,
  onClose,
}: {
  branch: BranchMetadata;
  project: string;
  onClose: () => void;
}) {
  return (
    <div
      className="card"
      style={{
        width: 320,
        flexShrink: 0,
        padding: 16,
        display: "flex",
        flexDirection: "column",
        gap: 10,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
        <span
          className="mono"
          style={{ fontSize: 13, color: "var(--fg-0)", flex: 1, minWidth: 0 }}
        >
          {branch.name ?? branch.id ?? "(unnamed)"}
        </span>
        <Tag tone={statusTone(branch.status)}>{branch.status ?? "unknown"}</Tag>
        <button
          type="button"
          onClick={onClose}
          style={{
            background: "transparent",
            border: 0,
            color: "var(--fg-3)",
            cursor: "pointer",
            fontSize: 14,
            lineHeight: 1,
            padding: "2px 4px",
          }}
          title="Close detail panel"
        >
          ×
        </button>
      </div>

      {branch.fork_valid_time ? (
        <DetailRow label="Fork point" value={branch.fork_valid_time} />
      ) : null}
      {branch.merge_strategy ? (
        <DetailRow label="Merge strategy" value={branch.merge_strategy} />
      ) : null}
      {branch.merge_point_event_id ? (
        <DetailRow label="Merge event" value={branch.merge_point_event_id} mono />
      ) : null}
      {branch.abandonment_reason ? (
        <DetailRow label="Abandon reason" value={branch.abandonment_reason} />
      ) : null}
      {branch.created_at ? (
        <DetailRow label="Created" value={branch.created_at.slice(0, 10)} />
      ) : null}
      {branch.created_by ? (
        <DetailRow label="Created by" value={branch.created_by} />
      ) : null}
      {branch.parent_branch_id ? (
        <DetailRow label="Parent branch" value={branch.parent_branch_id} mono />
      ) : null}

      <button
        type="button"
        className="btn btn--ghost"
        style={{ marginTop: 8, fontSize: 11.5 }}
        onClick={() =>
          filterTimelineBranch(project, branch.name ?? branch.id ?? "")
        }
        title="Filter Bitemporal Timeline to this branch"
      >
        Filter timeline to this branch
      </button>
    </div>
  );
}

function DetailRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <span style={{ fontSize: 10, color: "var(--fg-3)", textTransform: "uppercase", letterSpacing: "0.04em" }}>
        {label}
      </span>
      <span
        className={mono ? "mono" : undefined}
        style={{ fontSize: 11.5, color: "var(--fg-1)", wordBreak: "break-all" }}
      >
        {value}
      </span>
    </div>
  );
}

function Empty({ msg }: { msg: string }) {
  return (
    <div
      style={{
        marginTop: 24,
        padding: 32,
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
