use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use cortex_local::{
    Authority, CaptureRequest, LocalError, LocalStore, RevisionRequest, SearchQuery, Source,
};
use tempfile::TempDir;

const WRITERS: usize = 4;
const RECORDS_PER_WRITER: usize = 12;

fn request(id: String, body: String) -> CaptureRequest {
    CaptureRequest {
        id: Some(id),
        repository: "concurrent-project".into(),
        title: "Concurrent durable evidence".into(),
        body,
        source: Source {
            kind: "test".into(),
            locator: "concurrency-test".into(),
            captured_at: Some("2026-10-17T12:00:00Z".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

// Real processes ensure the durable lock works beyond a shared Rust mutex.
#[test]
fn subprocess_writer() {
    let Some(root) = std::env::var_os("CORTEX_LOCAL_TEST_WRITER_ROOT") else {
        return;
    };
    let writer: usize = std::env::var("CORTEX_LOCAL_TEST_WRITER_ID")
        .unwrap()
        .parse()
        .unwrap();
    let root = Path::new(&root);
    std::fs::write(root.join(format!("ready-{writer}")), b"ready").unwrap();
    wait_for(&root.join("start"));
    let store = LocalStore::open(root.join("store")).unwrap();
    let conflict_winner = match store.capture(request(
        "contested-identity".into(),
        format!("exclusive evidence from writer {writer}"),
    )) {
        Ok(outcome) => {
            assert!(outcome.created);
            true
        }
        Err(LocalError::Conflict(_)) => false,
        Err(error) => panic!("unexpected competing capture failure: {error}"),
    };
    let mut shared_created = 0;
    for number in 0..RECORDS_PER_WRITER {
        let unique = store
            .capture(request(
                format!("writer-{writer}-{number}"),
                format!("parallelmarker writer {writer} item {number}"),
            ))
            .unwrap();
        assert!(unique.created);
        let shared = store
            .capture(request(
                format!("shared-{number}"),
                format!("parallelmarker shared item {number}"),
            ))
            .unwrap();
        shared_created += usize::from(shared.created);
    }
    std::fs::write(
        root.join(format!("result-{writer}")),
        shared_created.to_string(),
    )
    .unwrap();
    std::fs::write(
        root.join(format!("winner-{writer}")),
        usize::from(conflict_winner).to_string(),
    )
    .unwrap();
}

struct Children(Vec<Child>);

impl Drop for Children {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn independent_processes_preserve_unique_records_and_deduplicate_shared_records() {
    let root = TempDir::new().unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut children = Children(Vec::new());
    for writer in 0..WRITERS {
        children.0.push(
            Command::new(&executable)
                .args(["--exact", "subprocess_writer", "--nocapture"])
                .env("CORTEX_LOCAL_TEST_WRITER_ROOT", root.path())
                .env("CORTEX_LOCAL_TEST_WRITER_ID", writer.to_string())
                .stdin(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    for writer in 0..WRITERS {
        wait_for(&root.path().join(format!("ready-{writer}")));
    }
    std::fs::write(root.path().join("start"), b"start").unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    for child in &mut children.0 {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "writer exited with {status}");
                break;
            }
            assert!(Instant::now() < deadline, "concurrent writers stalled");
            thread::sleep(Duration::from_millis(10));
        }
    }
    let shared_created: usize = (0..WRITERS)
        .map(|writer| {
            std::fs::read_to_string(root.path().join(format!("result-{writer}")))
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .sum();
    assert_eq!(shared_created, RECORDS_PER_WRITER);
    let winners: Vec<usize> = (0..WRITERS)
        .filter(|writer| {
            std::fs::read_to_string(root.path().join(format!("winner-{writer}"))).unwrap() == "1"
        })
        .collect();
    assert_eq!(winners.len(), 1);

    let store = LocalStore::open(root.path().join("store")).unwrap();
    let contested = store
        .get("concurrent-project", "contested-identity")
        .unwrap()
        .unwrap();
    assert_eq!(
        contested.body,
        format!("exclusive evidence from writer {}", winners[0])
    );
    assert_eq!(contested.revision, 1);
    for writer in 0..WRITERS {
        for number in 0..RECORDS_PER_WRITER {
            let record = store
                .get("concurrent-project", &format!("writer-{writer}-{number}"))
                .unwrap()
                .unwrap();
            assert_eq!(
                record.body,
                format!("parallelmarker writer {writer} item {number}")
            );
            assert_eq!(record.revision, 1);
        }
    }
    for number in 0..RECORDS_PER_WRITER {
        assert_eq!(
            store
                .get("concurrent-project", &format!("shared-{number}"))
                .unwrap()
                .unwrap()
                .revision,
            1
        );
    }
    let query = || SearchQuery {
        repository: "concurrent-project".into(),
        text: "parallelmarker".into(),
        limit: 100,
        include_inactive: true,
    };
    let expected = (WRITERS + 1) * RECORDS_PER_WRITER;
    assert_eq!(store.search(query()).unwrap().len(), expected);
    store.reindex().unwrap();
    assert_eq!(store.search(query()).unwrap().len(), expected);
}

#[test]
fn competing_revisions_have_one_winner_without_lost_updates() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    store
        .capture(request("revision-race".into(), "original evidence".into()))
        .unwrap();
    let start = std::sync::Arc::new(std::sync::Barrier::new(WRITERS));
    let workers: Vec<_> = (0..WRITERS)
        .map(|writer| {
            let start = start.clone();
            let path = root.path().to_path_buf();
            thread::spawn(move || {
                let opened = LocalStore::open(path);
                start.wait();
                let store = opened.unwrap();
                let result = store.revise(
                    "concurrent-project",
                    "revision-race",
                    1,
                    RevisionRequest {
                        title: "Concurrent correction".into(),
                        body: format!("correction from writer {writer}"),
                        authority: Authority::Accepted,
                        links: Vec::new(),
                        metadata: serde_json::Value::Null,
                    },
                );
                match result {
                    Ok(outcome) => {
                        assert_eq!(outcome.record.revision, 2);
                        Some(writer)
                    }
                    Err(LocalError::Conflict(_)) => None,
                    Err(error) => panic!("unexpected revision failure: {error}"),
                }
            })
        })
        .collect();
    let winners: Vec<_> = workers
        .into_iter()
        .filter_map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(winners.len(), 1);
    let record = store
        .get("concurrent-project", "revision-race")
        .unwrap()
        .unwrap();
    assert_eq!(record.revision, 2);
    assert_eq!(
        record.body,
        format!("correction from writer {}", winners[0])
    );
    store.reindex().unwrap();
    assert_eq!(
        store
            .get("concurrent-project", "revision-race")
            .unwrap()
            .unwrap(),
        record
    );
}

#[test]
fn subprocess_lock_holder() {
    let Some(root) = std::env::var_os("CORTEX_LOCAL_TEST_LOCK_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("store.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    std::fs::write(root.join("pending"), b"interrupted writer").unwrap();
    std::fs::write(root.join("lock-ready"), b"ready").unwrap();
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn killed_process_releases_lock_and_pending_recovery_preserves_records() {
    let root = TempDir::new().unwrap();
    let store = LocalStore::open(root.path()).unwrap();
    let original = store
        .capture(request("survivor".into(), "durable evidence".into()))
        .unwrap()
        .record;
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "subprocess_lock_holder", "--nocapture"])
        .env("CORTEX_LOCAL_TEST_LOCK_ROOT", root.path())
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let mut children = Children(vec![child]);
    wait_for(&root.path().join("lock-ready"));
    let started = Instant::now();
    let blocked = LocalStore::open_with_lock_timeout(root.path(), Duration::from_millis(50));
    assert!(matches!(
        blocked,
        Err(LocalError::Io(ref error)) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
    assert!(started.elapsed() < Duration::from_secs(5));

    children.0[0].kill().unwrap();
    children.0[0].wait().unwrap();
    let recovered =
        LocalStore::open_with_lock_timeout(root.path(), Duration::from_secs(2)).unwrap();
    assert!(!root.path().join("pending").exists());
    assert_eq!(
        recovered.get("concurrent-project", "survivor").unwrap(),
        Some(original)
    );
    assert_eq!(
        recovered
            .status(Some("concurrent-project"))
            .unwrap()
            .records,
        1
    );
}
