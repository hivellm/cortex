//! Standalone command entry. Every transport calls the same local engine.
use clap::{Parser, Subcommand};
use cortex_local::{
    classification::{queue::DrainOptions, ClassificationQueue, Provider, DEFAULT_MODEL},
    import::{import_archive, import_daily, ImportOptions},
    runtime, AuditKind, AuditListQuery, AuditMetricsQuery, AuditRetentionRequest, Authority,
    CaptureRequest, GraphQuery, LocalError, LocalStore, RecordKind, Result, RevisionRequest,
    SearchQuery, Source,
};
use serde::Serialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Parser)]
#[command(
    name = "cortex",
    version,
    about = "Local memory, history and relationships. No background stack required."
)]
struct Cli {
    /// Cortex home; the canonical store lives in its local/ directory.
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Exact repository ID. Otherwise read [cortex] id from the working tree.
    #[arg(long, alias = "repo", global = true)]
    repository: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Count local records. --all explicitly includes every repository.
    Status {
        #[arg(long)]
        all: bool,
    },
    /// Capture a memory, or a complete JSON request from --input FILE (use - for stdin).
    Capture {
        #[arg(long, conflicts_with_all=["title","body","kind","authority"])]
        input: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        authority: Option<String>,
    },
    /// Inspect a record or an exact historical revision.
    Get {
        id: String,
        #[arg(long)]
        revision: Option<u64>,
    },
    /// Create a revision with an explicit expected-revision check.
    Revise {
        id: String,
        #[arg(long)]
        expected: u64,
        #[arg(long, default_value = "-")]
        input: PathBuf,
    },
    Search {
        text: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        include_inactive: bool,
    },
    Recent {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        include_inactive: bool,
    },
    Graph {
        id: String,
        #[arg(long, default_value_t = 1)]
        depth: usize,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Import historical data without modifying its source. Repeat to resume.
    Import {
        #[arg(value_parser=["daily","archive"])]
        format: String,
        source: PathBuf,
        #[arg(long, default_value = "legacy")]
        namespace: String,
        #[arg(long, default_value_t = 100_000)]
        max_files: usize,
        #[arg(long, default_value_t = 10_000)]
        max_records: usize,
        #[arg(long = "map", value_name = "ORIGINAL=TARGET")]
        mappings: Vec<String>,
        #[arg(long)]
        unscoped_repository: Option<String>,
        #[arg(long)]
        global_repository: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Rebuild derived indexes. --repair preserves a corrupt index in a local backup.
    Reindex {
        #[arg(long)]
        repair: bool,
    },
    /// Explicit advisory Jev work. Only `classify run` contacts TypeSafe.
    Classify {
        #[command(subcommand)]
        action: ClassificationCommand,
    },
    /// Inspect local delivery, action and citation evidence or preview archival.
    Audit {
        #[command(subcommand)]
        action: AuditCommand,
    },
    /// Run the embedded web interface on loopback; no other service is needed.
    Serve {
        #[arg(long, default_value_t = 17200)]
        port: u16,
    },
    /// Run the local MCP stdio transport, or inspect its implemented descriptors.
    Mcp {
        #[arg(long)]
        list_tools: bool,
    },
    /// Lifecycle transport: bounded stdin and fail-open JSON output.
    Hook {
        #[arg(long, default_value = "claude-code")]
        client: String,
        #[arg(long)]
        event: Option<String>,
        #[arg(long)]
        fire_forget: bool,
    },
    /// Inspect this executable and local store. Does not start legacy services.
    Doctor,
    /// Identify this executable without opening or creating a store.
    BuildInfo,
    /// Install this executable and its embedded plugins into Cortex home.
    Install,
    /// Install an explicitly supplied standalone executable and its own assets.
    Update {
        #[arg(long)]
        binary: PathBuf,
    },
}

#[derive(Subcommand)]
enum AuditCommand {
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        before: Option<String>,
        #[arg(long, value_parser=["delivery", "observation", "archive_control"])]
        kind: Option<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        include_archived: bool,
    },
    Get {
        id: String,
    },
    Metrics {
        #[arg(long, default_value_t = 10_000)]
        limit: usize,
        #[arg(long)]
        after: Option<String>,
        #[arg(long)]
        before: Option<String>,
        #[arg(long)]
        include_archived: bool,
    },
    /// Preview affected audit families; --apply preserves files and archives visibility.
    Retain {
        #[arg(long)]
        cutoff: String,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Subcommand)]
enum ClassificationCommand {
    Enqueue {
        id: String,
        #[arg(long,default_value=DEFAULT_MODEL)]
        model: String,
    },
    Jobs {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Get {
        key: String,
    },
    Retry {
        key: String,
    },
    /// Send queued redacted candidates to TypeSafe using TYPESAFE_API_KEY.
    Run {
        #[arg(long, default_value_t = 20)]
        max_jobs: usize,
        #[arg(long, default_value_t = 30)]
        seconds: u64,
    },
}

fn print(value: &impl Serialize) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    Ok(())
}

fn invalid(message: &str) -> LocalError {
    LocalError::Invalid(message.into())
}

async fn run(cli: Cli) -> Result<()> {
    if matches!(cli.command, Command::BuildInfo) {
        return print(&json!({
            "runtime":"standalone", "version":env!("CARGO_PKG_VERSION"),
            "install_protocol":1, "release":!cfg!(debug_assertions),
            "required_services":[]
        }));
    }
    // Hooks must always produce accepted client JSON, including configuration/read failures.
    if let Command::Hook {
        client,
        event,
        fire_forget,
    } = &cli.command
    {
        let response = (|| {
            let root = runtime::home(cli.home.as_deref())?.join("local");
            let input = runtime::read_json(Path::new("-"), 1024 * 1024)?;
            Ok::<_, LocalError>(cortex_local::hooks::run(
                &root,
                client,
                event.as_deref(),
                input,
            ))
        })()
        .unwrap_or_else(|_| json!({}));
        return if *fire_forget {
            print(&json!({}))
        } else {
            print(&response)
        };
    }
    if matches!(cli.command, Command::Mcp { list_tools: true }) {
        return print(&cortex_local::mcp::descriptors());
    }
    let home = runtime::home(cli.home.as_deref())?;
    let root = home.join("local");
    if let Command::Update { binary } = &cli.command {
        return print(&cortex_local::upgrade::update(&home, binary)?);
    }
    if matches!(cli.command, Command::Install) {
        let report = cortex_local::install::install(&home, &std::env::current_exe()?)?;
        let root = std::fs::canonicalize(&home)?.join("local");
        // Initialize only a missing store; upgrading never repairs existing data.
        match std::fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                LocalStore::open(&root)?;
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        return print(&report);
    }
    if matches!(cli.command, Command::Reindex { repair: true }) {
        return print(&LocalStore::repair(&root)?);
    }
    let dry_run = matches!(cli.command, Command::Import { dry_run: true, .. });
    let temporary = if dry_run {
        match std::fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Some(tempfile::tempdir()?)
            }
            Err(error) => return Err(error.into()),
            Ok(_) => None,
        }
    } else {
        None
    };
    let store = if let Some(temporary) = &temporary {
        LocalStore::open(temporary.path())?
    } else if dry_run {
        LocalStore::inspect(&root)?
    } else {
        LocalStore::open(&root)?
    };
    let cwd = std::env::current_dir()?;
    let scope = || runtime::resolve_repository(&cwd, cli.repository.as_deref());
    match cli.command {
        Command::Status { all } => {
            if all && cli.repository.is_some() {
                return Err(invalid("--all conflicts with --repository"));
            }
            let repository = if all { None } else { Some(scope()?) };
            print(&store.status(repository.as_deref())?)
        }
        Command::Capture {
            input,
            title,
            body,
            kind,
            authority,
        } => {
            let mut request: CaptureRequest = if let Some(input) = input {
                serde_json::from_value(runtime::read_json(
                    &input,
                    cortex_local::model::MAX_RECORD_BYTES,
                )?)?
            } else {
                CaptureRequest {
                    title: title.unwrap_or_else(|| "Memory".into()),
                    body: body.ok_or_else(|| invalid("capture requires --body or --input"))?,
                    kind: serde_json::from_value(json!(kind.unwrap_or_else(|| "note".into())))?,
                    authority: serde_json::from_value(json!(
                        authority.unwrap_or_else(|| "proposed".into())
                    ))?,
                    source: Source {
                        kind: "manual".into(),
                        locator: "cortex:cli".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            };
            if request.repository.is_empty() {
                request.repository = scope()?;
            }
            if cli
                .repository
                .as_ref()
                .is_some_and(|repository| repository != &request.repository)
            {
                return Err(invalid("input repository differs from --repository"));
            }
            if request.kind == RecordKind::Event && request.authority != Authority::Observation {
                return Err(invalid("event capture requires observation authority"));
            }
            print(&store.capture(request)?)
        }
        Command::Get { id, revision } => {
            let repository = scope()?;
            let record = match revision {
                Some(revision) => store.get_revision(&repository, &id, revision)?,
                None => store.get(&repository, &id)?,
            };
            print(&record.ok_or_else(|| LocalError::NotFound(id))?)
        }
        Command::Revise {
            id,
            expected,
            input,
        } => {
            let request: RevisionRequest = serde_json::from_value(runtime::read_json(
                &input,
                cortex_local::model::MAX_RECORD_BYTES,
            )?)?;
            print(&store.revise(&scope()?, &id, expected, request)?)
        }
        Command::Search {
            text,
            limit,
            include_inactive,
        } => print(&store.search(SearchQuery {
            repository: scope()?,
            text,
            limit,
            include_inactive,
        })?),
        Command::Recent {
            limit,
            include_inactive,
        } => print(&store.recent(&scope()?, limit, include_inactive)?),
        Command::Graph { id, depth, limit } => print(&store.graph(GraphQuery {
            repository: scope()?,
            id,
            depth,
            limit,
        })?),
        Command::Import {
            format,
            source,
            namespace,
            max_files,
            max_records,
            mappings,
            unscoped_repository,
            global_repository,
            dry_run,
        } => {
            let mut repository_map = BTreeMap::new();
            for mapping in mappings {
                let (original, target) = mapping
                    .split_once('=')
                    .ok_or_else(|| invalid("repository mapping must be ORIGINAL=TARGET"))?;
                if repository_map
                    .insert(original.to_owned(), target.to_owned())
                    .is_some()
                {
                    return Err(invalid("duplicate source repository mapping"));
                }
            }
            let options = ImportOptions {
                namespace,
                repository_map,
                unscoped_repository,
                global_repository,
                max_files,
                max_records,
                dry_run,
            };
            let report = if format == "daily" {
                import_daily(&store, &source, &options)?
            } else {
                import_archive(&store, &source, &options)?
            };
            print(&report)
        }
        Command::Reindex { .. } => print(&store.reindex()?),
        Command::Audit { action } => {
            let repository = scope()?;
            match action {
                AuditCommand::List {
                    limit,
                    after,
                    before,
                    kind,
                    cursor,
                    include_archived,
                } => {
                    let kind = kind.map(|kind| match kind.as_str() {
                        "delivery" => AuditKind::Delivery,
                        "observation" => AuditKind::Observation,
                        _ => AuditKind::ArchiveControl,
                    });
                    print(&store.list_audit(AuditListQuery {
                        repository,
                        limit,
                        after,
                        before,
                        kind,
                        cursor,
                        include_archived,
                    })?)
                }
                AuditCommand::Get { id } => {
                    let record = store
                        .get_audit(&repository, &id)?
                        .ok_or_else(|| LocalError::NotFound(id))?;
                    print(&record)
                }
                AuditCommand::Metrics {
                    limit,
                    after,
                    before,
                    include_archived,
                } => print(&store.audit_metrics(AuditMetricsQuery {
                    repository,
                    after,
                    before,
                    include_archived,
                    limit,
                })?),
                AuditCommand::Retain {
                    cutoff,
                    limit,
                    apply,
                } => print(&store.retain_audit(AuditRetentionRequest {
                    repository,
                    cutoff,
                    limit,
                    apply,
                })?),
            }
        }
        Command::Classify { action } => {
            let repository = scope()?;
            let queue = ClassificationQueue::new(store);
            match action {
                ClassificationCommand::Enqueue { id, model } => {
                    print(&queue.enqueue(&repository, &id, &model)?)
                }
                ClassificationCommand::Jobs { limit } => print(&queue.list(&repository, limit)?),
                ClassificationCommand::Get { key } => print(&queue.get(&repository, &key)?),
                ClassificationCommand::Retry { key } => print(&queue.retry(&repository, &key)?),
                ClassificationCommand::Run { max_jobs, seconds } => {
                    let provider = Provider::new(runtime::typesafe_key(), Duration::from_secs(10))?;
                    print(
                        &queue
                            .drain(
                                &provider,
                                DrainOptions {
                                    repository,
                                    max_jobs,
                                    deadline: Duration::from_secs(seconds),
                                },
                            )
                            .await?,
                    )
                }
            }
        }
        Command::Serve { port } => cortex_local::web::serve(store, port, scope().ok()).await,
        Command::Mcp { .. } => cortex_local::mcp::run(store, scope().ok()),
        Command::Doctor => print(
            &json!({"version":env!("CARGO_PKG_VERSION"),"executable":std::env::current_exe()?,"home":home,"store":store.root(),"status":store.status(None)?,"runtime":"standalone","required_services":[],"classification_configured":runtime::typesafe_key().is_some()}),
        ),
        Command::Hook { .. } | Command::BuildInfo | Command::Install | Command::Update { .. } => {
            unreachable!("handled before opening storage")
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("cortex: {error}");
        std::process::exit(1);
    }
}
