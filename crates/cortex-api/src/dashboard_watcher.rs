//! File-system watcher feeding the dashboard event bus (spec 21).
//!
//! Watches `.rulebook/{tasks,handoff,decisions,knowledge}/**` for changes
//! and emits [`cortex_core::DashboardEvent`]s into a shared
//! [`DashboardEventBus`]. The MCP server is the primary publisher; this
//! watcher is the fallback that catches manual edits, git checkouts, and
//! anything else that bypasses the MCP boundary.
//!
//! ## Why `notify-debouncer-mini`
//!
//! Editors (and our own tooling) write a file in two passes:
//! `tasks.md.tmp` → rename. `notify` raises three events for that.
//! Debouncing on a 250 ms window collapses the burst into one event per
//! path so we publish once per logical change.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, DebouncedEventKind};
use tokio::sync::broadcast;

use cortex_core::{DashboardEvent, DashboardEventKind, DashboardEventSource};

/// Default debounce window for file-system events. Covers the
/// "write tmp + rename" pattern most editors use.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(250);

/// Capacity of the broadcast channel. Each subscriber gets a slot; events
/// are dropped (oldest first) when a subscriber lags. The dashboard
/// stream contract documents the lossy behaviour and how the GUI recovers
/// (full `invalidateQueries` on the next `hello`).
pub const BUS_CAPACITY: usize = 1024;

/// Shared, cloneable bus carrying [`DashboardEvent`]s from every
/// publisher (watcher + MCP consumer) to every SSE subscriber.
#[derive(Clone)]
pub struct DashboardEventBus {
    tx: broadcast::Sender<DashboardEvent>,
}

impl DashboardEventBus {
    /// Build a fresh bus with the default capacity. Lossy on lag.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }

    /// Publish one event. Returns the number of active subscribers that
    /// received it — `0` is fine and means nobody is listening yet
    /// (events are not buffered for late subscribers).
    pub fn publish(&self, event: DashboardEvent) -> usize {
        // `send` only errors when there are zero subscribers. That is
        // not a failure for us; a quiet daemon with no GUI is normal.
        self.tx.send(event).unwrap_or(0)
    }

    /// Open a fresh subscription. Each subscriber sees only events
    /// published after the call.
    pub fn subscribe(&self) -> broadcast::Receiver<DashboardEvent> {
        self.tx.subscribe()
    }
}

impl Default for DashboardEventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Entry-points the watcher recursively monitors, relative to the
/// `.rulebook/` root.
const WATCH_SUBDIRS: &[&str] = &["tasks", "handoff", "decisions", "knowledge", "learnings"];

/// Spawn the file-system watcher and return a handle that keeps the
/// watcher alive. Drop the handle to stop watching.
///
/// `root` is the absolute path to the project's `.rulebook/` directory.
/// Missing subdirectories are skipped silently — the dashboard is happy
/// to run on a partially-populated workspace.
///
/// Errors when the underlying [`notify`] backend fails to register any
/// watch. On that path the caller can keep running without the watcher;
/// the MCP publisher path will still feed the bus.
pub fn spawn_watcher(root: PathBuf, bus: DashboardEventBus) -> notify::Result<WatcherHandle> {
    let root_for_classify = root.clone();
    let bus_for_thread = bus.clone();
    let mut debouncer = new_debouncer(DEFAULT_DEBOUNCE, move |res: DebounceEventResult| {
        if let Ok(events) = res {
            for ev in events {
                if !matches!(ev.kind, DebouncedEventKind::Any) {
                    continue;
                }
                if let Some(dashboard_event) = classify(&root_for_classify, &ev.path) {
                    bus_for_thread.publish(dashboard_event);
                }
            }
        }
    })?;

    let mut watched_any = false;
    for sub in WATCH_SUBDIRS {
        let path = root.join(sub);
        if path.exists() {
            debouncer
                .watcher()
                .watch(&path, RecursiveMode::Recursive)?;
            watched_any = true;
        }
    }

    if !watched_any {
        return Err(notify::Error::generic(
            "no .rulebook subdirectories present to watch",
        ));
    }

    Ok(WatcherHandle {
        _debouncer: Arc::new(debouncer),
    })
}

/// Owns the debouncer; drop = stop watching.
pub struct WatcherHandle {
    _debouncer: Arc<dyn std::any::Any + Send + Sync>,
}

/// Map a changed path to its dashboard event kind. Returns `None` when
/// the path is uninteresting (e.g. lockfile, hidden temp).
pub fn classify(root: &Path, path: &Path) -> Option<DashboardEvent> {
    let rel = path.strip_prefix(root).ok()?;
    let mut comps = rel.components();
    let top = comps.next()?.as_os_str().to_str()?;
    let (kind, entity_id) = match top {
        "tasks" => {
            // .rulebook/tasks/<id>/{tasks,proposal}.md
            let id = comps.next()?.as_os_str().to_str()?;
            (DashboardEventKind::TaskChanged, id.to_string())
        }
        "handoff" => {
            // .rulebook/handoff/<file>
            let file = comps.next()?.as_os_str().to_str()?;
            (DashboardEventKind::HandoffAppended, file.to_string())
        }
        "decisions" => {
            // .rulebook/decisions/<id>.md
            let file = comps.next()?.as_os_str().to_str()?;
            let id = file.strip_suffix(".md").unwrap_or(file).to_string();
            (DashboardEventKind::DecisionChanged, id)
        }
        "knowledge" => {
            let file = comps.next()?.as_os_str().to_str()?;
            let id = file.strip_suffix(".md").unwrap_or(file).to_string();
            (DashboardEventKind::KnowledgeAdded, id)
        }
        "learnings" => {
            // .rulebook/learnings/<slug>.md (sibling .metadata.json fires
            // its own event; both round-trip through the same dedup ring).
            let file = comps.next()?.as_os_str().to_str()?;
            let id = file
                .strip_suffix(".metadata.json")
                .or_else(|| file.strip_suffix(".md"))
                .unwrap_or(file)
                .to_string();
            (DashboardEventKind::LearningAdded, id)
        }
        _ => return None,
    };
    Some(DashboardEvent {
        event_id: cortex_core::event_id().to_string(),
        kind,
        entity_id,
        summary: None,
        ts: chrono::Utc::now().to_rfc3339(),
        delta: None,
        source: DashboardEventSource::Watcher,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classify_task_change_extracts_id() {
        let root = Path::new("/proj/.rulebook");
        let p = root.join("tasks/phase11m_x/tasks.md");
        let ev = classify(root, &p).expect("matched");
        assert!(matches!(ev.kind, DashboardEventKind::TaskChanged));
        assert_eq!(ev.entity_id, "phase11m_x");
        assert!(matches!(ev.source, DashboardEventSource::Watcher));
    }

    #[test]
    fn classify_handoff_uses_filename() {
        let root = Path::new("/proj/.rulebook");
        let p = root.join("handoff/_pending.md");
        let ev = classify(root, &p).expect("matched");
        assert!(matches!(ev.kind, DashboardEventKind::HandoffAppended));
        assert_eq!(ev.entity_id, "_pending.md");
    }

    #[test]
    fn classify_decision_strips_md_suffix() {
        let root = Path::new("/proj/.rulebook");
        let p = root.join("decisions/DEC-0042.md");
        let ev = classify(root, &p).expect("matched");
        assert!(matches!(ev.kind, DashboardEventKind::DecisionChanged));
        assert_eq!(ev.entity_id, "DEC-0042");
    }

    #[test]
    fn classify_knowledge_strips_md_suffix() {
        let root = Path::new("/proj/.rulebook");
        let p = root.join("knowledge/pattern_42.md");
        let ev = classify(root, &p).expect("matched");
        assert!(matches!(ev.kind, DashboardEventKind::KnowledgeAdded));
        assert_eq!(ev.entity_id, "pattern_42");
    }

    #[test]
    fn classify_learning_strips_md_suffix() {
        let root = Path::new("/proj/.rulebook");
        let p = root.join("learnings/2026-05-03T05-00-00-some-insight.md");
        let ev = classify(root, &p).expect("matched");
        assert!(matches!(ev.kind, DashboardEventKind::LearningAdded));
        assert_eq!(ev.entity_id, "2026-05-03T05-00-00-some-insight");
    }

    #[test]
    fn classify_learning_strips_metadata_json_suffix() {
        let root = Path::new("/proj/.rulebook");
        let p = root.join("learnings/2026-05-03T05-00-00-foo.metadata.json");
        let ev = classify(root, &p).expect("matched");
        assert!(matches!(ev.kind, DashboardEventKind::LearningAdded));
        assert_eq!(ev.entity_id, "2026-05-03T05-00-00-foo");
    }

    #[test]
    fn classify_unrelated_returns_none() {
        let root = Path::new("/proj/.rulebook");
        let p = root.join("specs/RULEBOOK.md");
        assert!(classify(root, &p).is_none());
    }

    #[test]
    fn classify_outside_root_returns_none() {
        let root = Path::new("/proj/.rulebook");
        let p = Path::new("/other/place/tasks.md");
        assert!(classify(root, p).is_none());
    }

    #[tokio::test]
    async fn bus_delivers_published_event_to_subscriber() {
        let bus = DashboardEventBus::new();
        let mut rx = bus.subscribe();
        let event = DashboardEvent {
            event_id: "01J".to_string(),
            kind: DashboardEventKind::TaskChanged,
            entity_id: "phase1_x".to_string(),
            summary: None,
            ts: "2026-05-02T00:00:00Z".to_string(),
            delta: None,
            source: DashboardEventSource::Watcher,
        };
        let n = bus.publish(event.clone());
        assert_eq!(n, 1, "one subscriber received the event");
        let received = rx.recv().await.expect("event delivered");
        assert_eq!(received, event);
    }

    #[tokio::test]
    async fn bus_publish_with_no_subscribers_is_silent() {
        let bus = DashboardEventBus::new();
        let event = DashboardEvent {
            event_id: "01J".to_string(),
            kind: DashboardEventKind::TaskChanged,
            entity_id: "phase1_x".to_string(),
            summary: None,
            ts: "2026-05-02T00:00:00Z".to_string(),
            delta: None,
            source: DashboardEventSource::Watcher,
        };
        let n = bus.publish(event);
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn watcher_emits_event_on_real_file_write() {
        let tmp = tempfile::tempdir().expect("tmp dir");
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join("tasks/phase1_demo")).expect("mkdir");

        let bus = DashboardEventBus::new();
        let mut rx = bus.subscribe();
        let _handle = spawn_watcher(root.clone(), bus.clone()).expect("watcher");

        // Give the OS-level watch a moment to register before writing.
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::fs::write(
            root.join("tasks/phase1_demo/tasks.md"),
            "## 1.\n- [ ] 1.1 todo\n",
        )
        .expect("write");

        let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("event arrived in time")
            .expect("event payload");
        assert!(matches!(received.kind, DashboardEventKind::TaskChanged));
        assert_eq!(received.entity_id, "phase1_demo");
    }
}
