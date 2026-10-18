export type LocalView = "timeline" | "memory" | "decisions" | "classifications" | "graph" | "health" | "audit" | "feedback";
export interface LocalRecord {
  id: string; repository: string; revision: number; kind: string; authority: string;
  title: string; body: string; created_at: string; updated_at: string; content_sha256: string;
  source: { kind: string; locator: string; source_hash?: string | null; [key: string]: unknown };
  links: { target_id: string; relation: string }[]; metadata: unknown;
}
export interface AuditReference { id: string; repository: string; revision: number; content_sha256: string; [key: string]: unknown }
export interface AuditRecord {
  id: string; repository: string; recorded_at: string; evidence_label: string;
  payload: { type: string; content: { query_id?: string; delivery_id?: string; references?: AuditReference[]; evidence?: AuditReference[]; event?: AuditReference; [key: string]: unknown } };
}
export interface FeedbackRequest {
  id: string; repository: string; query_id?: string; delivery_id: string; reference?: AuditReference;
  source: { kind: string; reference: string; actor: string }; evidence: "explicit";
  judgment: string; comment: string;
}
export interface FeedbackEntry { record: { request: FeedbackRequest; recorded_at: string }; eligibility: string }
