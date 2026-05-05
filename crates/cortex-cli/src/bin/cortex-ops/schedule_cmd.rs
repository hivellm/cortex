use std::process::ExitCode;
use super::{ScheduleCommand, helpers::home_dir};

/// Phase9k — `cortex-ops schedule` dispatcher. Each subcommand
/// translates straight onto `cortex_cli::ops::scheduler` calls plus
/// a small JSON / plain-text renderer.
pub(super) fn schedule(command: ScheduleCommand) -> ExitCode {
    use cortex_cli::ops::scheduler::{
        next_after, parse_schedule, run_now, seed_defaults, tick, MemoryRunner, ProcessRunner,
        Scheduler,
    };
    use cortex_storage::MetadataStore;

    fn resolve_db(arg: Option<String>) -> std::path::PathBuf {
        arg.map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("CORTEX_METADATA_DB")
                    .ok()
                    .map(std::path::PathBuf::from)
            })
            .unwrap_or_else(|| {
                home_dir()
                    .map(|h| h.join(".cortex").join("metadata.sqlite"))
                    .unwrap_or_else(|| std::path::PathBuf::from(".cortex/metadata.sqlite"))
            })
    }
    fn open(arg: Option<String>) -> Result<MetadataStore, ExitCode> {
        let path = resolve_db(arg);
        match MetadataStore::open(&path) {
            Ok(s) => Ok(s),
            Err(e) => {
                eprintln!("metadata open ({}): {e}", path.display());
                Err(ExitCode::FAILURE)
            }
        }
    }
    fn build_runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => Ok(rt),
            Err(e) => {
                eprintln!("tokio runtime: {e}");
                Err(ExitCode::FAILURE)
            }
        }
    }
    match command {
        ScheduleCommand::List { json, metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let jobs = match store.list_cron_jobs() {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("schedule list: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if json {
                let arr: Vec<serde_json::Value> = jobs
                    .iter()
                    .map(|j| {
                        serde_json::json!({
                            "name": j.name,
                            "schedule": j.schedule,
                            "command": j.command,
                            "enabled": j.enabled,
                            "next_run_at": j.next_run_at,
                            "last_run_at": j.last_run_at,
                            "last_status": j.last_status,
                            "failure_streak": j.failure_streak,
                        })
                    })
                    .collect();
                match serde_json::to_string_pretty(&arr) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("serialize: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                println!(
                    "{:<32} {:<14} {:<8} {:<25} {:<10}",
                    "name", "schedule", "enabled", "next_run_at", "last_status"
                );
                for j in &jobs {
                    println!(
                        "{:<32} {:<14} {:<8} {:<25} {:<10}",
                        j.name,
                        j.schedule,
                        if j.enabled { "yes" } else { "no" },
                        j.next_run_at.clone().unwrap_or_else(|| "—".to_string()),
                        j.last_status.clone().unwrap_or_else(|| "—".to_string()),
                    );
                }
            }
            ExitCode::SUCCESS
        }
        ScheduleCommand::Show {
            name,
            json,
            metadata_db,
        } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let job = match store.get_cron_job(&name) {
                Ok(Some(j)) => j,
                Ok(None) => {
                    eprintln!("unknown job: {name}");
                    return ExitCode::FAILURE;
                }
                Err(e) => {
                    eprintln!("schedule show: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if json {
                let payload = serde_json::json!({
                    "name": job.name,
                    "schedule": job.schedule,
                    "command": job.command,
                    "enabled": job.enabled,
                    "last_run_at": job.last_run_at,
                    "last_status": job.last_status,
                    "next_run_at": job.next_run_at,
                    "last_error": job.last_error,
                    "last_stdout": job.last_stdout,
                    "last_stderr": job.last_stderr,
                    "failure_streak": job.failure_streak,
                    "last_warning_at": job.last_warning_at,
                });
                match serde_json::to_string_pretty(&payload) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("serialize: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                println!("name:           {}", job.name);
                println!("schedule:       {}", job.schedule);
                println!("command:        {}", job.command);
                println!("enabled:        {}", job.enabled);
                println!("next_run_at:    {}", job.next_run_at.unwrap_or_default());
                println!("last_run_at:    {}", job.last_run_at.unwrap_or_default());
                println!("last_status:    {}", job.last_status.unwrap_or_default());
                println!("failure_streak: {}", job.failure_streak);
                if let Some(err) = job.last_error {
                    println!("last_error:     {err}");
                }
                if let Some(out) = job.last_stdout {
                    println!("--- stdout tail ---\n{out}");
                }
                if let Some(err) = job.last_stderr {
                    println!("--- stderr tail ---\n{err}");
                }
            }
            ExitCode::SUCCESS
        }
        ScheduleCommand::Enable { name, metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            match store.set_cron_job_enabled(&name, true) {
                Ok(0) => {
                    eprintln!("unknown job: {name}");
                    ExitCode::FAILURE
                }
                Ok(_) => {
                    println!("{name}: enabled");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("schedule enable: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::Disable { name, metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            match store.set_cron_job_enabled(&name, false) {
                Ok(0) => {
                    eprintln!("unknown job: {name}");
                    ExitCode::FAILURE
                }
                Ok(_) => {
                    println!("{name}: disabled");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("schedule disable: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::Set {
            name,
            cron,
            metadata_db,
        } => {
            if let Err(e) = parse_schedule(&cron) {
                eprintln!("invalid cron: {e}");
                return ExitCode::FAILURE;
            }
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let now = chrono::Utc::now();
            let next = match next_after(&cron, now) {
                Some(t) => t.to_rfc3339(),
                None => {
                    eprintln!("cron: no upcoming run derived from {cron:?}");
                    return ExitCode::FAILURE;
                }
            };
            match store.set_cron_job_schedule(&name, &cron, &next) {
                Ok(0) => {
                    eprintln!("unknown job: {name}");
                    ExitCode::FAILURE
                }
                Ok(_) => {
                    println!("{name}: schedule={cron} next_run_at={next}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("schedule set: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::RunNow { name, metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let runner = ProcessRunner;
            let scheduler = Scheduler::new();
            let runtime = match build_runtime() {
                Ok(rt) => rt,
                Err(c) => return c,
            };
            let now = chrono::Utc::now();
            match runtime.block_on(run_now(&scheduler, &runner, &store, &name, now)) {
                Ok(out) => {
                    println!("{name}: {} ", out.status);
                    if let Some(err) = out.last_error {
                        eprintln!("last_error: {err}");
                    }
                    match out.status.as_str() {
                        "success" => ExitCode::SUCCESS,
                        "lock_held" => ExitCode::from(2),
                        _ => ExitCode::FAILURE,
                    }
                }
                Err(e) => {
                    eprintln!("schedule run-now: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::SeedDefaults { metadata_db } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            match seed_defaults(&store, chrono::Utc::now()) {
                Ok(n) => {
                    println!(
                        "seeded {n} default jobs ({} total)",
                        store.list_cron_jobs().map(|j| j.len()).unwrap_or(0)
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("seed-defaults: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        ScheduleCommand::Tick {
            metadata_db,
            time_travel,
        } => {
            let store = match open(metadata_db) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let now = match time_travel {
                Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
                    Ok(t) => t.with_timezone(&chrono::Utc),
                    Err(e) => {
                        eprintln!("--time-travel parse error: {e}");
                        return ExitCode::FAILURE;
                    }
                },
                None => chrono::Utc::now(),
            };
            // For ad-hoc tick invocation we use a fresh in-process
            // MemoryRunner so the tick is observable without
            // double-spawning real children — operators that want
            // to actually trigger sweeps should use `run-now`.
            let runtime = match build_runtime() {
                Ok(rt) => rt,
                Err(c) => return c,
            };
            let runner = MemoryRunner::new();
            let scheduler = Scheduler::new();
            match runtime.block_on(tick(&scheduler, &runner, &store, now)) {
                Ok(report) => {
                    println!(
                        "tick: due={} success={} failed={} lock_held={}",
                        report.due, report.successes, report.failures, report.lock_held
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("schedule tick: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
