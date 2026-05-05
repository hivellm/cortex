#!/usr/bin/env python3
"""
graph_correlate.py — offline cross-source correlation analyzer.

Walks every HiveLLM project, every Rulebook artifact (tasks, decisions,
knowledge, learnings, laws), every Claude Code transcript, and every git
log. Emits a single Cypher script (idempotent MERGE statements) that
materialises the resulting node+edge graph.

The output file is intentionally plain text so it can be diffed,
grep'd, and replayed against a throwaway Nexus / Neo4j instance for
sanity-checking before any production ingestion is attempted.

stdlib only. Tested on Python 3.10+.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable, Iterator

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------

DEFAULT_ROOT = Path("E:/HiveLLM")
DEFAULT_TRANSCRIPTS = Path("C:/Users/Bolado/.claude/projects")
DEFAULT_OUT = Path("E:/HiveLLM/Cortex/scripts/graph/graph_correlate.cypher")
DEFAULT_STATS = Path("E:/HiveLLM/Cortex/scripts/graph/graph_correlate.stats.txt")

# Per-source caps — protect output size on huge repos. -1 = unlimited.
DEFAULT_GIT_LIMIT = 2000          # commits per repo
DEFAULT_TRANSCRIPT_LIMIT = -1     # transcript files per project (-1 = all)
DEFAULT_TURN_LIMIT_PER_FILE = -1  # entries per jsonl

# ---------------------------------------------------------------------------
# Regex extractors — used to surface implicit references inside free text.
# ---------------------------------------------------------------------------

# phase11u_cortex-forget-mcp-tool, phase0_event-schema, etc.
RX_TASK_ID = re.compile(r"\bphase\d+[a-z]*_[a-z0-9][a-z0-9_-]+", re.IGNORECASE)

# DEC-0042 or decision-001 or decision 001 (loose; we filter against known set later)
RX_DECISION = re.compile(r"\b(?:DEC-?|decision[ -])(\d{1,4})\b", re.IGNORECASE)

# §3 / §3.2 / §3.2.1
RX_SECTION = re.compile(r"§\d+(?:\.\d+)*")

# LAW-CORTEX-001, LAW-RULEBOOK-002, ...
RX_LAW = re.compile(r"\bLAW-[A-Z][A-Z0-9]+-\d+\b")

# 7-40 hex chars, word-bounded — filtered later against known shas.
RX_SHA = re.compile(r"\b[0-9a-f]{7,40}\b")

# Conventional commit footer: "task: <id>" or "Task: <id>"
RX_COMMIT_TASK_FOOTER = re.compile(
    r"^(?:task|tasks|closes?|fixes?|implements?)\s*:\s*([a-z0-9_][a-z0-9_-]+)",
    re.IGNORECASE | re.MULTILINE,
)

# ---------------------------------------------------------------------------
# Secret redaction — applied to every user-prose field before it lands in the
# Cypher dump. Mirrors the pattern catalog in `cortex-core::redact`. Adding
# defense-in-depth here is mandatory because transcripts/commits frequently
# carry pasted tokens that the operator typed into a prompt.
# Real incident reference: PyPI/NuGet/Vectorizer keys leaked in v1 of this
# script (commit 1ec6a8a) — see SECURITY.md / `git log scripts/graph/`.
# ---------------------------------------------------------------------------

SECRET_PATTERNS: list[tuple[str, re.Pattern]] = [
    ("pypi_token", re.compile(r"pypi-[A-Za-z0-9_-]{16,}")),
    ("nuget_api_key", re.compile(r"\boy2[a-z0-9]{40,60}\b", re.IGNORECASE)),
    ("github_pat", re.compile(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}\b")),
    ("github_fine_grained_pat", re.compile(r"\bgithub_pat_[A-Za-z0-9_]{60,}\b")),
    ("anthropic_key", re.compile(r"\bsk-ant-[A-Za-z0-9_-]{20,}\b")),
    ("openai_key", re.compile(r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}\b")),
    ("aws_access_key", re.compile(r"\b(?:AKIA|ASIA|AGPA|AROA|AIPA|ANPA|ANVA|ASCA)[0-9A-Z]{16}\b")),
    ("google_api_key", re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b")),
    ("slack_token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b")),
    ("hf_token", re.compile(r"\bhf_[A-Za-z0-9]{30,}\b")),
    ("jwt", re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b")),
    (
        "pem_private_key",
        re.compile(
            r"-----BEGIN (?:RSA |EC |OPENSSH |DSA |PGP )?PRIVATE KEY-----[\s\S]*?-----END (?:RSA |EC |OPENSSH |DSA |PGP )?PRIVATE KEY-----"
        ),
    ),
    ("bearer", re.compile(r"\bBearer\s+[A-Za-z0-9._\-]{20,}\b", re.IGNORECASE)),
]

# Catches "token <X>", "api_key=<X>", "password: <X>" — the most common shape
# in transcript prose where users paste a literal credential after a label.
SECRET_CONTEXT = re.compile(
    r"(?i)(\b(?:token|key|secret|password|passwd|pwd|api[_-]?key|api[_-]?token|bearer)\b)"
    r"\s*[:=]?\s*([\"'`]?)([A-Za-z0-9._/+=\-]{16,})\2"
)

# High-entropy fallback: mixed-case alphanumeric runs of length ≥ 28 that
# contain at least one upper, one lower, one digit. Hex SHAs are all
# lowercase so they don't trip this; ULIDs are uppercase-only so they don't
# either. Targets standalone API-key shapes like the Vectorizer one
# (`wAToQ6CpmbhXXMGHaLAf3osAsGomJJcm`) that the contextual rule misses
# when prose words separate the label from the value.
HIGH_ENTROPY_TOKEN = re.compile(
    r"\b(?=[A-Za-z0-9]*[A-Z])(?=[A-Za-z0-9]*[a-z])(?=[A-Za-z0-9]*[0-9])"
    r"[A-Za-z0-9]{28,}\b"
)


def redact(text: str | None) -> str:
    if text is None:
        return ""
    s = str(text)
    for cls, rx in SECRET_PATTERNS:
        s = rx.sub(f"[REDACTED:{cls}]", s)
    s = SECRET_CONTEXT.sub(lambda m: f"{m.group(1)} [REDACTED:contextual_secret]", s)
    s = HIGH_ENTROPY_TOKEN.sub("[REDACTED:high_entropy_token]", s)
    return s

# ---------------------------------------------------------------------------
# Cypher escape helpers
# ---------------------------------------------------------------------------


def cy_str(s: str | None, max_len: int = 400) -> str:
    """Escape arbitrary string for safe inclusion in a Cypher literal."""
    if s is None:
        return '""'
    s = str(s)
    if len(s) > max_len:
        s = s[: max_len - 1] + "..."
    s = s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", " ").replace("\r", " ")
    return f'"{s}"'


def cy_props(props: dict) -> str:
    out = []
    for k, v in props.items():
        if v is None:
            continue
        if isinstance(v, bool):
            out.append(f"{k}: {'true' if v else 'false'}")
        elif isinstance(v, (int, float)):
            out.append(f"{k}: {v}")
        elif isinstance(v, list):
            inner = ", ".join(cy_str(x) for x in v)
            out.append(f"{k}: [{inner}]")
        else:
            out.append(f"{k}: {cy_str(v)}")
    return "{" + ", ".join(out) + "}"


# ---------------------------------------------------------------------------
# Graph emitter — buffers everything in memory, dedups by id, then writes.
# ---------------------------------------------------------------------------


class Graph:
    def __init__(self, label_prefix: str = "") -> None:
        # nodes: { (label, id) -> dict of props }
        self.nodes: dict[tuple[str, str], dict] = {}
        # edges: { (label, src_id, src_label, dst_id, dst_label) -> dict of props }
        self.edges: dict[tuple[str, str, str, str, str], dict] = {}
        self.warnings: list[str] = []
        self.label_prefix = label_prefix

    def _lbl(self, raw: str) -> str:
        return f"{self.label_prefix}{raw}" if self.label_prefix else raw

    def add_node(self, label: str, node_id: str, **props) -> str:
        key = (label, node_id)
        existing = self.nodes.setdefault(key, {})
        for k, v in props.items():
            if v is None:
                continue
            existing[k] = v
        existing["id"] = node_id
        return node_id

    def add_edge(
        self,
        rel: str,
        src_label: str,
        src_id: str,
        dst_label: str,
        dst_id: str,
        **props,
    ) -> None:
        key = (rel, src_label, src_id, dst_label, dst_id)
        existing = self.edges.setdefault(key, {})
        for k, v in props.items():
            if v is None:
                continue
            existing[k] = v

    def warn(self, msg: str) -> None:
        self.warnings.append(msg)

    def stats(self) -> dict:
        node_by_label: Counter = Counter()
        for (label, _), _ in self.nodes.items():
            node_by_label[label] += 1
        edge_by_rel: Counter = Counter()
        for (rel, *_), _ in self.edges.items():
            edge_by_rel[rel] += 1
        return {
            "nodes_total": len(self.nodes),
            "edges_total": len(self.edges),
            "nodes_by_label": dict(node_by_label),
            "edges_by_rel": dict(edge_by_rel),
            "warnings": len(self.warnings),
        }

    def write_cypher(self, path: Path, header: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("w", encoding="utf-8") as f:
            f.write(header)
            f.write("\n// ============================================================\n")
            f.write("// NODES\n")
            f.write("// ============================================================\n\n")

            # Group nodes by label for readability
            by_label: dict[str, list[tuple[str, dict]]] = defaultdict(list)
            for (label, nid), props in self.nodes.items():
                by_label[label].append((nid, props))

            for label in sorted(by_label):
                rendered = self._lbl(label)
                f.write(f"// --- {rendered} ({len(by_label[label])}) ---\n")
                for nid, props in sorted(by_label[label], key=lambda x: x[0]):
                    f.write(f"MERGE (n:{rendered} {{id: {cy_str(nid)}}}) SET n += {cy_props(props)};\n")
                f.write("\n")

            f.write("// ============================================================\n")
            f.write("// EDGES\n")
            f.write("// ============================================================\n\n")

            by_rel: dict[str, list] = defaultdict(list)
            for (rel, sl, sid, dl, did), props in self.edges.items():
                by_rel[rel].append((sl, sid, dl, did, props))

            for rel in sorted(by_rel):
                rendered_rel = self._lbl(rel)
                f.write(f"// --- :{rendered_rel} ({len(by_rel[rel])}) ---\n")
                for sl, sid, dl, did, props in sorted(by_rel[rel], key=lambda x: (x[0], x[1], x[2], x[3])):
                    prop_clause = f" SET r += {cy_props(props)}" if props else ""
                    f.write(
                        f"MATCH (s:{self._lbl(sl)} {{id: {cy_str(sid)}}}), (d:{self._lbl(dl)} {{id: {cy_str(did)}}}) "
                        f"MERGE (s)-[r:{rendered_rel}]->(d){prop_clause};\n"
                    )
                f.write("\n")

            if self.warnings:
                f.write("// ============================================================\n")
                f.write("// WARNINGS\n")
                f.write("// ============================================================\n")
                for w in self.warnings:
                    f.write(f"// {w}\n")


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


def find_projects(root: Path) -> list[Path]:
    if not root.exists():
        return []
    projects = []
    for child in sorted(root.iterdir()):
        if not child.is_dir():
            continue
        if (child / ".rulebook").is_dir() or (child / ".git").is_dir():
            projects.append(child)
    return projects


def project_id(p: Path) -> str:
    return f"repo:{p.name}"


# ---------------------------------------------------------------------------
# Rulebook ingestion
# ---------------------------------------------------------------------------


def read_text(p: Path, max_bytes: int = 1_000_000) -> str:
    try:
        with p.open("r", encoding="utf-8", errors="replace") as f:
            return f.read(max_bytes)
    except OSError:
        return ""


def read_json(p: Path) -> dict | None:
    try:
        with p.open("r", encoding="utf-8") as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return None


def parse_tasks_md(content: str) -> tuple[int, int, list[str]]:
    """Return (total, checked, item_titles)."""
    items = []
    checked = 0
    total = 0
    for line in content.splitlines():
        m = re.match(r"^\s*-\s*\[([ xX])\]\s*(.*)$", line)
        if not m:
            continue
        total += 1
        if m.group(1).lower() == "x":
            checked += 1
        items.append(m.group(2).strip())
    return total, checked, items


def ingest_task_dir(g: Graph, repo: Path, task_dir: Path, archived: bool) -> None:
    task_id = task_dir.name
    if archived:
        # archive folder name: 2026-04-18-phase0_cortex-core
        m = re.match(r"^\d{4}-\d{2}-\d{2}-(.+)$", task_id)
        if m:
            task_id = m.group(1)

    full_id = f"task:{repo.name}/{task_id}"
    tasks_md = read_text(task_dir / "tasks.md")
    proposal_md = read_text(task_dir / "proposal.md")
    total, checked, items = parse_tasks_md(tasks_md)

    g.add_node(
        "Task",
        full_id,
        repo=repo.name,
        task_id=task_id,
        archived=archived,
        items_total=total,
        items_done=checked,
        progress=(round(100.0 * checked / total, 1) if total else 0.0),
    )
    g.add_edge("IN_REPO", "Task", full_id, "Repo", project_id(repo))

    # Item nodes — preserve exact done state per line (re-parse to get the flag)
    seq = 0
    for line in tasks_md.splitlines():
        m = re.match(r"^\s*-\s*\[([ xX])\]\s*(.*)$", line)
        if not m:
            continue
        seq += 1
        done = m.group(1).lower() == "x"
        title = m.group(2).strip()
        item_id = f"{full_id}#item-{seq}"
        g.add_node(
            "TaskItem",
            item_id,
            task=task_id,
            seq=seq,
            title=redact(title)[:200],
            done=done,
        )
        g.add_edge("HAS_ITEM", "Task", full_id, "TaskItem", item_id, seq=seq)

    # References mined from proposal.md text — task↔decision/law/section/other-task
    for ref_task in set(RX_TASK_ID.findall(proposal_md)):
        if ref_task == task_id:
            continue
        ref_id = f"task:{repo.name}/{ref_task}"
        g.add_node("Task", ref_id, repo=repo.name, task_id=ref_task, _stub=True)
        g.add_edge("REFERENCES", "Task", full_id, "Task", ref_id, source="proposal_md")

    for law_id in set(RX_LAW.findall(proposal_md)):
        g.add_node("Law", f"law:{law_id}", law_id=law_id, _stub=True)
        g.add_edge("MENTIONS", "Task", full_id, "Law", f"law:{law_id}", source="proposal_md")


def ingest_decisions(g: Graph, repo: Path) -> None:
    dec_dir = repo / ".rulebook" / "decisions"
    if not dec_dir.is_dir():
        return
    for meta in dec_dir.glob("*.metadata.json"):
        data = read_json(meta) or {}
        slug = data.get("slug") or meta.stem.replace(".metadata", "")
        dec_id = f"decision:{repo.name}/{data.get('id', slug)}"
        body = read_text(meta.with_suffix("").with_suffix(".md"))

        g.add_node(
            "Decision",
            dec_id,
            repo=repo.name,
            decision_id=data.get("id"),
            slug=slug,
            title=redact(data.get("title")),
            status=data.get("status"),
            date=data.get("date"),
        )
        g.add_edge("IN_REPO", "Decision", dec_id, "Repo", project_id(repo))

        if data.get("supersededBy"):
            sup_id = f"decision:{repo.name}/{data['supersededBy']}"
            g.add_node("Decision", sup_id, repo=repo.name, decision_id=data["supersededBy"], _stub=True)
            g.add_edge("SUPERSEDED_BY", "Decision", dec_id, "Decision", sup_id)

        for rt in data.get("relatedTasks") or []:
            t_id = f"task:{repo.name}/{rt}"
            g.add_node("Task", t_id, repo=repo.name, task_id=rt, _stub=True)
            g.add_edge("RESOLVES", "Decision", dec_id, "Task", t_id, source="metadata.relatedTasks")

        # Also mine the markdown body for cross-refs the metadata didn't capture
        for ref_task in set(RX_TASK_ID.findall(body)):
            t_id = f"task:{repo.name}/{ref_task}"
            g.add_node("Task", t_id, repo=repo.name, task_id=ref_task, _stub=True)
            g.add_edge("MENTIONS", "Decision", dec_id, "Task", t_id, source="decision_md")
        for law_id in set(RX_LAW.findall(body)):
            g.add_node("Law", f"law:{law_id}", law_id=law_id, _stub=True)
            g.add_edge("MENTIONS", "Decision", dec_id, "Law", f"law:{law_id}", source="decision_md")


def ingest_learnings(g: Graph, repo: Path) -> None:
    learn_dir = repo / ".rulebook" / "learnings"
    if not learn_dir.is_dir():
        return
    for meta in learn_dir.glob("*.metadata.json"):
        data = read_json(meta) or {}
        learn_id = f"learning:{repo.name}/{data.get('id', meta.stem)}"
        g.add_node(
            "Learning",
            learn_id,
            repo=repo.name,
            title=redact(data.get("title")),
            source=data.get("source"),
            tags=data.get("tags"),
            createdAt=data.get("createdAt"),
        )
        g.add_edge("IN_REPO", "Learning", learn_id, "Repo", project_id(repo))
        if data.get("relatedTask"):
            t_id = f"task:{repo.name}/{data['relatedTask']}"
            g.add_node("Task", t_id, repo=repo.name, task_id=data["relatedTask"], _stub=True)
            g.add_edge("EXTRACTED_FROM_TASK", "Learning", learn_id, "Task", t_id)


def ingest_knowledge(g: Graph, repo: Path) -> None:
    base = repo / ".rulebook" / "knowledge"
    if not base.is_dir():
        return
    for kind in ("patterns", "anti-patterns"):
        for md in (base / kind).glob("*.md") if (base / kind).is_dir() else []:
            kid = f"knowledge:{repo.name}/{kind}/{md.stem}"
            body = read_text(md)
            title_match = re.search(r"^#\s+(.+)$", body, re.MULTILINE)
            g.add_node(
                "Knowledge",
                kid,
                repo=repo.name,
                kind=kind,
                slug=md.stem,
                title=redact(title_match.group(1) if title_match else md.stem),
            )
            g.add_edge("IN_REPO", "Knowledge", kid, "Repo", project_id(repo))
            for ref_task in set(RX_TASK_ID.findall(body)):
                t_id = f"task:{repo.name}/{ref_task}"
                g.add_node("Task", t_id, repo=repo.name, task_id=ref_task, _stub=True)
                g.add_edge("MENTIONS", "Knowledge", kid, "Task", t_id, source="knowledge_md")


def ingest_laws(g: Graph, repo: Path) -> None:
    """Laws live in AGENTS.override.md as `LAW-<SCOPE>-<NNN>` headings."""
    override = repo / "AGENTS.override.md"
    if not override.exists():
        return
    body = read_text(override)
    for m in re.finditer(
        r"##\s+(LAW-[A-Z][A-Z0-9]+-\d+)\s+—\s+(.+?)$",
        body,
        re.MULTILINE,
    ):
        law_id = m.group(1)
        g.add_node(
            "Law",
            f"law:{law_id}",
            law_id=law_id,
            title=redact(m.group(2).strip()),
            repo=repo.name,
        )
        g.add_edge("IN_REPO", "Law", f"law:{law_id}", "Repo", project_id(repo))


# ---------------------------------------------------------------------------
# Git ingestion
# ---------------------------------------------------------------------------


def ingest_git(g: Graph, repo: Path, limit: int) -> None:
    if not (repo / ".git").is_dir():
        return
    # Two-pass approach to avoid commit-body newlines polluting the file list:
    # 1) metadata via %x1e-separated records, fields delimited by %x1f
    # 2) file paths per commit via a separate `git log --name-only` invocation
    try:
        meta_out = subprocess.run(
            [
                "git",
                "-C",
                str(repo),
                "log",
                f"-n{limit}" if limit > 0 else "--all",
                "--pretty=format:%H%x1f%an%x1f%ae%x1f%aI%x1f%s%x1f%b%x1e",
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=120,
        )
        files_out = subprocess.run(
            [
                "git",
                "-C",
                str(repo),
                "log",
                f"-n{limit}" if limit > 0 else "--all",
                "--name-only",
                "--pretty=format:__SHA__%H",
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=120,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired) as e:
        g.warn(f"git log failed for {repo.name}: {e}")
        return
    if meta_out.returncode != 0:
        g.warn(f"git log rc={meta_out.returncode} for {repo.name}: {meta_out.stderr[:200]}")
        return

    # Build sha -> [paths] map from second invocation
    files_by_sha: dict[str, list[str]] = {}
    current_sha: str | None = None
    for line in files_out.stdout.splitlines():
        if line.startswith("__SHA__"):
            current_sha = line[len("__SHA__"):].strip()
            files_by_sha.setdefault(current_sha, [])
        elif current_sha and line.strip():
            files_by_sha[current_sha].append(line.strip())

    # Records separated by \x1e, fields by \x1f.
    for rec in meta_out.stdout.split("\x1e"):
        rec = rec.strip("\n\r ")
        if not rec:
            continue
        parts = rec.split("\x1f")
        if len(parts) < 5:
            continue
        sha, an, ae, aiso, subject = parts[0], parts[1], parts[2], parts[3], parts[4]
        body = parts[5] if len(parts) > 5 else ""
        commit_id = f"commit:{repo.name}/{sha}"
        g.add_node(
            "Commit",
            commit_id,
            repo=repo.name,
            sha=sha,
            short=sha[:8],
            author=an,
            email=ae,
            date=aiso,
            subject=redact(subject)[:200],
        )
        g.add_edge("IN_REPO", "Commit", commit_id, "Repo", project_id(repo))

        # Mine subject + body for task/decision/law refs
        haystack = f"{subject}\n{body}"
        for tref in set(RX_TASK_ID.findall(haystack)):
            t_id = f"task:{repo.name}/{tref}"
            g.add_node("Task", t_id, repo=repo.name, task_id=tref, _stub=True)
            g.add_edge("REFERENCES_TASK", "Commit", commit_id, "Task", t_id)
        for foot in RX_COMMIT_TASK_FOOTER.findall(body):
            t_id = f"task:{repo.name}/{foot}"
            g.add_node("Task", t_id, repo=repo.name, task_id=foot, _stub=True)
            g.add_edge("CLOSES", "Commit", commit_id, "Task", t_id, source="commit_footer")
        for law_id in set(RX_LAW.findall(haystack)):
            g.add_node("Law", f"law:{law_id}", law_id=law_id, _stub=True)
            g.add_edge("MENTIONS", "Commit", commit_id, "Law", f"law:{law_id}")
        for sec in set(RX_SECTION.findall(haystack)):
            # Section refs become a property snapshot, not a node — too noisy.
            pass

        # Files touched (from second git log invocation)
        for path in files_by_sha.get(sha, []):
            art_id = f"artifact:{repo.name}/{path}"
            g.add_node("Artifact", art_id, repo=repo.name, path=path)
            g.add_edge("MODIFIES", "Commit", commit_id, "Artifact", art_id)
            g.add_edge("IN_REPO", "Artifact", art_id, "Repo", project_id(repo))


# ---------------------------------------------------------------------------
# Claude Code transcript ingestion
# ---------------------------------------------------------------------------

# Map a project workspace dir name (e--HiveLLM-Cortex) → repo name (Cortex)
TRANSCRIPT_DIR_RX = re.compile(r"^[a-z]--HiveLLM-(.+)$", re.IGNORECASE)


def project_repo_name_from_transcript_dir(name: str) -> str | None:
    m = TRANSCRIPT_DIR_RX.match(name)
    if m:
        return m.group(1)
    return None


def iter_jsonl(path: Path, limit: int) -> Iterator[dict]:
    try:
        with path.open("r", encoding="utf-8", errors="replace") as f:
            for i, line in enumerate(f):
                if limit > 0 and i >= limit:
                    break
                line = line.strip()
                if not line:
                    continue
                try:
                    yield json.loads(line)
                except json.JSONDecodeError:
                    continue
    except OSError:
        return


def extract_tool_paths(tool_input: dict) -> list[str]:
    """Best-effort extraction of file paths from tool_use input payloads."""
    paths: list[str] = []
    if not isinstance(tool_input, dict):
        return paths
    for key in ("file_path", "path", "notebook_path"):
        if key in tool_input and isinstance(tool_input[key], str):
            paths.append(tool_input[key])
    # Bash command: pull plausible file refs (contains / or \)
    cmd = tool_input.get("command")
    if isinstance(cmd, str):
        for tok in re.findall(r"[A-Za-z]:[\\/][^\s\"']+|[/\\][\w./\\-]+", cmd):
            if any(seg in tok for seg in (".", "/", "\\")):
                paths.append(tok)
    # Edit/Write content fields aren't paths — skip
    return paths


def normalize_path_for_repo(path: str, repo_root: Path) -> str | None:
    """Convert absolute path to repo-relative if it falls inside repo_root."""
    try:
        p = Path(path)
        if not p.is_absolute():
            return path.replace("\\", "/")
        rel = p.resolve().relative_to(repo_root.resolve())
        return str(rel).replace("\\", "/")
    except (ValueError, OSError):
        return None


def ingest_transcripts(
    g: Graph,
    transcripts_root: Path,
    repo_index: dict[str, Path],
    file_limit: int,
    turn_limit: int,
) -> None:
    if not transcripts_root.exists():
        return

    for proj_dir in sorted(transcripts_root.iterdir()):
        if not proj_dir.is_dir():
            continue
        repo_name = project_repo_name_from_transcript_dir(proj_dir.name)
        if not repo_name or repo_name not in repo_index:
            continue
        repo_root = repo_index[repo_name]

        jsonl_files = sorted(proj_dir.glob("*.jsonl"))
        if file_limit > 0:
            jsonl_files = jsonl_files[:file_limit]

        for jf in jsonl_files:
            session_id = jf.stem
            full_session = f"session:{session_id}"
            g.add_node(
                "Session",
                full_session,
                session_id=session_id,
                repo=repo_name,
                file=str(jf),
            )
            g.add_edge("IN_REPO", "Session", full_session, "Repo", project_id(repo_root))

            turn_count = 0
            tool_count = 0
            first_ts: str | None = None
            last_ts: str | None = None

            for entry in iter_jsonl(jf, turn_limit):
                ts = entry.get("timestamp")
                if isinstance(ts, str):
                    if first_ts is None:
                        first_ts = ts
                    last_ts = ts

                etype = entry.get("type")
                uuid = entry.get("uuid") or entry.get("toolUseID")
                if not uuid:
                    continue

                if etype == "user" or etype == "assistant":
                    turn_id = f"turn:{uuid}"
                    role = etype
                    msg = entry.get("message") or {}
                    content = msg.get("content") if isinstance(msg, dict) else None
                    text_parts: list[str] = []
                    if isinstance(content, list):
                        for c in content:
                            if isinstance(c, dict) and c.get("type") == "text":
                                t = c.get("text")
                                if isinstance(t, str):
                                    text_parts.append(t)
                            elif isinstance(c, dict) and c.get("type") == "tool_use":
                                # Will surface as ToolCall below
                                pass
                    elif isinstance(content, str):
                        text_parts.append(content)
                    text = " ".join(text_parts)

                    g.add_node(
                        "Turn",
                        turn_id,
                        session=session_id,
                        role=role,
                        ts=ts,
                        text_excerpt=redact(text)[:300],
                    )
                    g.add_edge("HAS_TURN", "Session", full_session, "Turn", turn_id)
                    turn_count += 1

                    # Mine references in the text
                    for tref in set(RX_TASK_ID.findall(text)):
                        t_id = f"task:{repo_name}/{tref}"
                        g.add_node("Task", t_id, repo=repo_name, task_id=tref, _stub=True)
                        g.add_edge("MENTIONS_TASK", "Turn", turn_id, "Task", t_id)
                    for law_id in set(RX_LAW.findall(text)):
                        g.add_node("Law", f"law:{law_id}", law_id=law_id, _stub=True)
                        g.add_edge("MENTIONS_LAW", "Turn", turn_id, "Law", f"law:{law_id}")
                    for sha in set(RX_SHA.findall(text)):
                        if len(sha) >= 7:
                            commit_id = f"commit:{repo_name}/{sha}"
                            # Don't auto-create — only link if the commit was already seen
                            if ("Commit", commit_id) in g.nodes:
                                g.add_edge("MENTIONS_COMMIT", "Turn", turn_id, "Commit", commit_id)

                    # Tool uses inline in assistant messages
                    if isinstance(content, list):
                        for c in content:
                            if isinstance(c, dict) and c.get("type") == "tool_use":
                                tu_id = c.get("id") or f"{uuid}#{c.get('name','tool')}"
                                tc_id = f"toolcall:{tu_id}"
                                tname = c.get("name", "tool")
                                tinput = c.get("input") or {}
                                g.add_node(
                                    "ToolCall",
                                    tc_id,
                                    session=session_id,
                                    tool=tname,
                                    ts=ts,
                                )
                                g.add_edge("INVOKED", "Turn", turn_id, "ToolCall", tc_id)
                                tool_count += 1
                                for raw_path in extract_tool_paths(tinput):
                                    rel = normalize_path_for_repo(raw_path, repo_root)
                                    if not rel:
                                        continue
                                    art_id = f"artifact:{repo_name}/{rel}"
                                    g.add_node("Artifact", art_id, repo=repo_name, path=rel, _stub=True)
                                    g.add_edge("TOUCHED", "ToolCall", tc_id, "Artifact", art_id)

            g.nodes[("Session", full_session)].update(
                {
                    "turns": turn_count,
                    "tool_calls": tool_count,
                    "first_ts": first_ts,
                    "last_ts": last_ts,
                }
            )


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------


def write_stats(stats: dict, path: Path, runtime_s: float, args) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        f.write(f"# graph_correlate.py — run summary\n")
        f.write(f"generated_at: {datetime.now(timezone.utc).isoformat()}\n")
        f.write(f"runtime_seconds: {runtime_s:.2f}\n")
        f.write(f"root: {args.root}\n")
        f.write(f"transcripts: {args.transcripts}\n")
        f.write(f"git_limit: {args.git_limit}\n")
        f.write(f"transcript_limit: {args.transcript_limit}\n")
        f.write(f"turn_limit: {args.turn_limit}\n")
        f.write("\n## Totals\n")
        f.write(f"nodes_total: {stats['nodes_total']}\n")
        f.write(f"edges_total: {stats['edges_total']}\n")
        f.write(f"warnings: {stats['warnings']}\n")
        f.write("\n## Nodes by label\n")
        for k, v in sorted(stats["nodes_by_label"].items(), key=lambda x: -x[1]):
            f.write(f"  {k}: {v}\n")
        f.write("\n## Edges by relation\n")
        for k, v in sorted(stats["edges_by_rel"].items(), key=lambda x: -x[1]):
            f.write(f"  {k}: {v}\n")


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="Cross-source graph correlation analyzer (offline).")
    p.add_argument("--root", type=Path, default=DEFAULT_ROOT, help="HiveLLM workspace root (default: %(default)s)")
    p.add_argument("--transcripts", type=Path, default=DEFAULT_TRANSCRIPTS, help="Claude Code transcripts root")
    p.add_argument("--out", type=Path, default=DEFAULT_OUT, help="Output Cypher file")
    p.add_argument("--stats", type=Path, default=DEFAULT_STATS, help="Output stats file")
    p.add_argument("--git-limit", type=int, default=DEFAULT_GIT_LIMIT, help="Commits per repo (-1 = all)")
    p.add_argument("--transcript-limit", type=int, default=DEFAULT_TRANSCRIPT_LIMIT, help="Transcript files per project")
    p.add_argument("--turn-limit", type=int, default=DEFAULT_TURN_LIMIT_PER_FILE, help="Entries per transcript file")
    p.add_argument("--only-repos", type=str, default="", help="Comma-separated repo names to include (default: all)")
    p.add_argument("--no-git", action="store_true", help="Skip git log ingestion")
    p.add_argument("--no-transcripts", action="store_true", help="Skip Claude transcript ingestion")
    p.add_argument(
        "--label-prefix",
        type=str,
        default="",
        help="Prefix every node label and edge type (e.g. 'Cor_') to isolate from a populated graph",
    )
    args = p.parse_args(argv)

    t0 = datetime.now()

    projects = find_projects(args.root)
    if args.only_repos:
        wanted = {x.strip() for x in args.only_repos.split(",") if x.strip()}
        projects = [p for p in projects if p.name in wanted]

    if not projects:
        print(f"[graph_correlate] no projects found under {args.root}", file=sys.stderr)
        return 1

    print(f"[graph_correlate] projects: {len(projects)} -> {[p.name for p in projects]}")

    g = Graph(label_prefix=args.label_prefix)
    repo_index: dict[str, Path] = {}

    for repo in projects:
        rid = project_id(repo)
        g.add_node("Repo", rid, name=repo.name, path=str(repo))
        repo_index[repo.name] = repo

        # Active tasks
        tasks_dir = repo / ".rulebook" / "tasks"
        if tasks_dir.is_dir():
            for child in sorted(tasks_dir.iterdir()):
                if child.is_dir() and (child / "tasks.md").exists():
                    ingest_task_dir(g, repo, child, archived=False)

        # Archived tasks
        archive_dir = repo / ".rulebook" / "archive"
        if archive_dir.is_dir():
            for child in sorted(archive_dir.iterdir()):
                if child.is_dir() and (child / "tasks.md").exists():
                    ingest_task_dir(g, repo, child, archived=True)

        ingest_decisions(g, repo)
        ingest_learnings(g, repo)
        ingest_knowledge(g, repo)
        ingest_laws(g, repo)

        if not args.no_git:
            print(f"[graph_correlate] git log {repo.name} (limit={args.git_limit})")
            ingest_git(g, repo, args.git_limit)

    if not args.no_transcripts:
        print(f"[graph_correlate] transcripts under {args.transcripts}")
        ingest_transcripts(
            g,
            args.transcripts,
            repo_index,
            args.transcript_limit,
            args.turn_limit,
        )

    runtime_s = (datetime.now() - t0).total_seconds()
    stats = g.stats()

    header = (
        f"// graph_correlate.cypher — generated {datetime.now(timezone.utc).isoformat()}\n"
        f"// root={args.root}  transcripts={args.transcripts}\n"
        f"// projects={[p.name for p in projects]}\n"
        f"// runtime_s={runtime_s:.2f}\n"
        f"// nodes={stats['nodes_total']}  edges={stats['edges_total']}\n"
        "//\n"
        "// All statements are idempotent (MERGE). Replay against a throwaway\n"
        "// Neo4j/Nexus instance for inspection. NOT meant for production ingest.\n"
    )
    g.write_cypher(args.out, header)
    write_stats(stats, args.stats, runtime_s, args)

    print(
        f"[graph_correlate] wrote {args.out}  nodes={stats['nodes_total']}  edges={stats['edges_total']}"
    )
    print(f"[graph_correlate] stats -> {args.stats}")
    if g.warnings:
        print(f"[graph_correlate] {len(g.warnings)} warnings (see Cypher tail)")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
