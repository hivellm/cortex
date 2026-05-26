# Domain Report + dashboard projection View (ADR-014)

**Category**: architecture
**Tags**: adr-014, dashboard, pure-reader, phase13f, report-view-pattern

## Description

Every dashboard-surfaced domain exposes a `*Report` struct (domain shape, written by the worker) and a `*ReportView` projection (handler shape, returned over the wire). The domain type stays free to evolve; the view freezes the wire contract. Dashboard handlers MUST call `report.view()` rather than render the Report directly — this keeps the wire format stable across domain-side refactors and is the structural enforcement for ADR-014's pure-reader rule.

## Example

// crates/cortex-workers/src/producer/report.rs
pub struct ProducerReport { /* domain fields */ }
pub struct ProducerReportView { /* wire fields + derived flags */ pub had_work: bool }
impl ProducerReport {
    pub fn view(&self) -> ProducerReportView { /* pure projection */ }
}
// dashboard handler:
async fn producers(...) -> Json&lt;ProducerReportView&gt; {
    Json(source.latest().await?.view())
}

## When to Use

When adding a new dashboard endpoint backed by a worker / cli domain. Mirror the Sweep pattern: `Domain::run() -> DomainReport`; `DomainReport::view() -> DomainReportView`; handler returns `Json(view)`. Pair with a `DomainReportSource` trait so the projection is testable without the live backend (Meili / SQLite / HTTP). Round-trip serde tests on both Report and View — caught a missing `Deserialize` derive on `ConsolidationRow` in phase13f.

## When NOT to Use

When the dashboard renders a derived metric that no domain owns (e.g. a sum across multiple Reports). For those, create a synthetic Report at the api layer with its own View — do NOT compute the metric inside the handler.
