//! `cortex update`: bring an installed Cortex up to the latest source.
//!
//! Dry run by default: prints the source checkout and its HEAD, the git and
//! cargo commands, the bin dir, the runtime processes that would be stopped and
//! the plugin refresh commands, and changes nothing. `--apply` runs, in order
//! and stopping at the first hard failure: `git pull --ff-only` (unless
//! `--no-pull`), `cargo build --release` of the installed binaries, the
//! `cortex install` copy into the bin dir, a stop of the old `cortex-api` /
//! `cortex-adapter-claude` running from the bin dir, the stale-aside cleanup
//! and a best-effort Claude Code / Codex plugin refresh.
//!
//! The runtime is not started here: the next synchronous hook finds the
//! adapter pipe absent and starts the new binaries (ADR 040). Every side
//! effect goes through [`Host`], so tests never run git, cargo or a plugin
//! CLI and never stop a real process.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::time::{Duration, Instant};

use cortex_adapter_claude_code::legacy::PLUGIN_ID;

use super::install_bins::{self, BINARIES};

/// Binaries the hook starts (ADR 040); `update` stops them so the next hook
/// starts the new build.
const RUNTIME: [&str; 2] = ["cortex-api", "cortex-adapter-claude"];
/// Owned by client sessions; never stopped, only counted.
const MCP_SERVER: &str = "cortex-mcp-server";
const MARKETPLACE: &str = "hivellm-cortex";
const STOP_WAIT: Duration = Duration::from_secs(5);
const KILL_WAIT: Duration = Duration::from_secs(2);

/// One external command, printed exactly as it is run.
#[derive(Debug, Clone, PartialEq)]
struct Cmd {
    program: &'static str,
    args: Vec<String>,
    cwd: Option<PathBuf>,
}

impl Cmd {
    fn new(program: &'static str, args: &[&str]) -> Self {
        Self {
            program,
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            cwd: None,
        }
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.program)?;
        for arg in &self.args {
            if arg.contains(char::is_whitespace) {
                write!(f, " \"{arg}\"")?;
            } else {
                write!(f, " {arg}")?;
            }
        }
        if let Some(cwd) = &self.cwd {
            write!(f, " (in {})", cwd.display())?;
        }
        Ok(())
    }
}

/// A running process and the executable image it runs.
#[derive(Debug, Clone, PartialEq)]
struct Proc {
    pid: u32,
    exe: PathBuf,
}

/// The side effects `update` needs. [`RealHost`] performs them; tests use a
/// recording fake.
trait Host {
    /// Resolve `program` on `PATH`.
    fn find(&self, program: &str) -> Option<PathBuf>;
    /// Run `cmd` with inherited stdio; `Ok` when it exits successfully.
    fn run(&mut self, cmd: &Cmd) -> Result<(), String>;
    /// Run `cmd` and return its stdout when it exits successfully.
    fn capture(&mut self, cmd: &Cmd) -> Option<String>;
    /// Every running process whose executable path is known.
    fn processes(&mut self) -> Vec<Proc>;
    /// Stop `targets` and wait briefly for them to exit.
    fn terminate(&mut self, targets: &[Proc]) -> Vec<(u32, Result<(), String>)>;
    /// Install the binaries from `from` into `to`.
    fn install(&mut self, from: &Path, to: &Path) -> Result<Vec<String>, String>;
    /// Remove stale `.old-*` asides from `dir`.
    fn remove_stale_asides(&mut self, dir: &Path) -> Vec<String>;
}

/// A plugin client CLI and the commands that refresh the Cortex plugin.
struct Client {
    program: &'static str,
    refresh: &'static [&'static [&'static str]],
}

const CLIENTS: [Client; 2] = [
    Client {
        program: "claude",
        refresh: &[
            &["plugin", "marketplace", "update", MARKETPLACE],
            &["plugin", "update", PLUGIN_ID],
        ],
    },
    // `codex plugin marketplace upgrade` refreshes Git snapshots only; for a
    // local marketplace, adding the plugin again re-copies it into the cache.
    Client {
        program: "codex",
        refresh: &[&["plugin", "add", PLUGIN_ID]],
    },
];

impl Client {
    fn list(&self) -> Cmd {
        Cmd::new(self.program, &["plugin", "list"])
    }

    fn refresh_cmds(&self) -> Vec<Cmd> {
        self.refresh
            .iter()
            .map(|args| Cmd::new(self.program, args))
            .collect()
    }
}

/// What `update` will do, resolved before anything runs.
#[derive(Debug)]
struct Plan {
    source: PathBuf,
    bin_dir: PathBuf,
    /// `bin_dir` canonicalized, for matching process image paths.
    canonical_bin_dir: PathBuf,
    target: PathBuf,
    pull: Option<Cmd>,
    build: Cmd,
}

/// Outcome of `--apply`, printed as the closing summary.
#[derive(Debug, Default)]
struct Report {
    head_before: Option<String>,
    head_after: Option<String>,
    changed: usize,
    unchanged: usize,
    stopped: Vec<u32>,
    stop_failures: Vec<String>,
    plugins: Vec<String>,
}

/// Entry point for `cortex update`.
pub(super) fn update(source: Option<PathBuf>, no_pull: bool, apply: bool) -> ExitCode {
    let Some(bin_dir) = install_bins::default_bin_dir() else {
        eprintln!("update: no Cortex home; set CORTEX_HOME");
        return ExitCode::FAILURE;
    };
    let mut host = RealHost {
        path: install_bins::path_dirs(),
    };
    let cargo_target_dir = std::env::var_os("CARGO_TARGET_DIR");
    match execute(
        &mut host,
        &bin_dir,
        source.as_deref(),
        no_pull,
        apply,
        cargo_target_dir,
    ) {
        Ok(None) => ExitCode::SUCCESS,
        Ok(Some(report)) => {
            print_summary(&report);
            if report.stop_failures.is_empty() {
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "update: installed, but {} old runtime process(es) could not be stopped",
                    report.stop_failures.len()
                );
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("update: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Resolve the plan, then print it (dry run) or carry it out (`apply`).
fn execute(
    host: &mut dyn Host,
    bin_dir: &Path,
    source: Option<&Path>,
    no_pull: bool,
    apply: bool,
    cargo_target_dir: Option<OsString>,
) -> Result<Option<Report>, String> {
    let source = resolve_source(source, bin_dir)?;
    let plan = Plan {
        canonical_bin_dir: bin_dir
            .canonicalize()
            .unwrap_or_else(|_| bin_dir.to_path_buf()),
        bin_dir: bin_dir.to_path_buf(),
        target: target_dir(&source, cargo_target_dir),
        pull: (!no_pull).then(|| git(&source, &["pull", "--ff-only"])),
        build: build_cmd(&source),
        source,
    };
    if apply {
        apply_plan(host, &plan).map(Some)
    } else {
        print_dry_run(host, &plan);
        Ok(None)
    }
}

fn is_workspace_root(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Cargo.toml"))
        .ok()
        .and_then(|raw| raw.parse::<toml::Table>().ok())
        .is_some_and(|table| table.contains_key("workspace"))
}

/// The nearest directory at or above `start` holding a workspace `Cargo.toml`.
fn workspace_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| is_workspace_root(dir))
        .map(Path::to_path_buf)
}

/// `--source`, else the workspace above the build dir recorded in the
/// install manifest.
fn resolve_source(explicit: Option<&Path>, bin_dir: &Path) -> Result<PathBuf, String> {
    if let Some(dir) = explicit {
        let dir = std::path::absolute(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        return if is_workspace_root(&dir) {
            Ok(dir)
        } else {
            Err(format!(
                "{} has no Cargo.toml with [workspace]; pass the root of the Cortex checkout",
                dir.display()
            ))
        };
    }
    let build = install_bins::manifest_source(bin_dir).ok_or_else(|| {
        format!(
            "no install manifest in {} records the source build; pass --source <Cortex checkout>",
            bin_dir.display()
        )
    })?;
    // Older installs recorded `--from` verbatim; a relative path depends on
    // the directory the install ran from, which is unknown here.
    if build.is_relative() {
        return Err(format!(
            "the install manifest in {} records a relative build dir ({}); pass --source <Cortex checkout>",
            bin_dir.display(),
            build.display()
        ));
    }
    workspace_root(&build).ok_or_else(|| {
        format!(
            "no Cargo workspace contains {} (from the install manifest); pass --source <Cortex checkout>",
            build.display()
        )
    })
}

/// Where cargo puts the release binaries: `$CARGO_TARGET_DIR/release` (a
/// relative value is taken from the checkout, cargo's working directory),
/// else `<source>/target/release`.
fn target_dir(source: &Path, cargo_target_dir: Option<OsString>) -> PathBuf {
    match cargo_target_dir.filter(|dir| !dir.is_empty()) {
        Some(dir) => source.join(dir).join("release"),
        None => source.join("target").join("release"),
    }
}

fn git(source: &Path, args: &[&str]) -> Cmd {
    let mut cmd = Cmd::new("git", &["-C"]);
    cmd.args.push(source.display().to_string());
    cmd.args.extend(args.iter().map(|arg| (*arg).to_string()));
    cmd
}

fn build_cmd(source: &Path) -> Cmd {
    let mut cmd = Cmd::new("cargo", &["build", "--release"]);
    for name in BINARIES {
        cmd.args.push("--bin".into());
        cmd.args.push(name.into());
    }
    cmd.cwd = Some(source.to_path_buf());
    cmd
}

fn head(host: &mut dyn Host, source: &Path) -> Option<String> {
    host.capture(&git(source, &["rev-parse", "--short", "HEAD"]))
        .map(|out| out.trim().to_string())
        .filter(|sha| !sha.is_empty())
}

/// Comparable form of a path: no `\\?\` prefix, no trailing separator, and
/// on Windows backslashes only and lower case.
fn normalize(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if cfg!(windows) {
        text.replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    } else {
        text.trim_end_matches('/').to_string()
    }
}

/// Processes running one of `names` (or a renamed `.old-*` aside of it, which
/// a running image follows) directly from `bin_dir`.
fn select(processes: &[Proc], bin_dir: &Path, names: &[&str]) -> Vec<Proc> {
    let bin_dir = normalize(bin_dir);
    processes
        .iter()
        .filter(|proc| {
            let (Some(parent), Some(file)) = (proc.exe.parent(), proc.exe.file_name()) else {
                return false;
            };
            if normalize(parent) != bin_dir {
                return false;
            }
            let file = file.to_string_lossy();
            let file = if cfg!(windows) {
                file.to_lowercase()
            } else {
                file.into_owned()
            };
            names.iter().any(|name| {
                let exe = install_bins::exe(name);
                file == exe || file.starts_with(&format!("{exe}.old-"))
            })
        })
        .cloned()
        .collect()
}

fn is_aside(proc: &Proc) -> bool {
    proc.exe
        .file_name()
        .is_some_and(|name| name.to_string_lossy().contains(".old-"))
}

/// Whether a `plugin list` output shows the Cortex plugin as installed.
/// Codex lists available plugins too, marked `not installed`.
fn plugin_installed(list: &str) -> bool {
    list.lines().any(|line| {
        line.split_whitespace().any(|token| token == PLUGIN_ID) && !line.contains("not installed")
    })
}

fn print_dry_run(host: &mut dyn Host, plan: &Plan) {
    let head = head(host, &plan.source).unwrap_or_else(|| "unknown".into());
    println!("source checkout: {} (HEAD {head})", plan.source.display());
    println!("bin dir: {}", plan.bin_dir.display());
    match &plan.pull {
        Some(pull) => println!("1. {pull}"),
        None => println!("1. git pull skipped (--no-pull)"),
    }
    println!("2. {}", plan.build);
    println!(
        "3. install {} -> {} (SHA-256 verified; running binaries are moved aside)",
        plan.target.display(),
        plan.bin_dir.display()
    );
    let processes = host.processes();
    let runtime = select(&processes, &plan.canonical_bin_dir, &RUNTIME);
    if runtime.is_empty() {
        println!("4. stop the old runtime: none running from the bin dir");
    } else {
        println!("4. stop the old runtime:");
        for proc in &runtime {
            println!("     pid {} {}", proc.pid, proc.exe.display());
        }
    }
    let mcp = select(&processes, &plan.canonical_bin_dir, &[MCP_SERVER]).len();
    println!(
        "   {mcp} {MCP_SERVER} process(es) from the bin dir are left running; each switches to the new binary on its client's next session"
    );
    println!("5. remove stale asides in {}", plan.bin_dir.display());
    println!("6. refresh plugins (best effort):");
    for client in &CLIENTS {
        match host.find(client.program) {
            None => println!("     {}: not on PATH; skipped", client.program),
            Some(path) => {
                let cmds: Vec<String> = client.refresh_cmds().iter().map(Cmd::to_string).collect();
                println!(
                    "     {} ({}): if `{}` shows {PLUGIN_ID}: {}",
                    client.program,
                    path.display(),
                    client.list(),
                    cmds.join("; ")
                );
            }
        }
    }
    println!(
        "the runtime is not started here: the next Claude Code or Codex hook starts the new one (ADR 040)"
    );
    println!("dry run: nothing changed; re-run with --apply to update");
}

fn apply_plan(host: &mut dyn Host, plan: &Plan) -> Result<Report, String> {
    let mut report = Report {
        head_before: head(host, &plan.source),
        ..Report::default()
    };
    if let Some(pull) = &plan.pull {
        println!("> {pull}");
        host.run(pull)
            .map_err(|e| format!("git pull failed: {e}; nothing was built or installed"))?;
    }
    println!("> {}", plan.build);
    host.run(&plan.build)
        .map_err(|e| format!("build failed: {e}; nothing was installed"))?;
    report.head_after = head(host, &plan.source);

    println!(
        "install: {} -> {}",
        plan.target.display(),
        plan.bin_dir.display()
    );
    let lines = host
        .install(&plan.target, &plan.bin_dir)
        .map_err(|e| format!("install failed: {e}; no process was stopped"))?;
    for line in &lines {
        println!("  {line}");
    }
    report.changed = lines
        .iter()
        .filter(|l| l.starts_with("installed: "))
        .count();
    report.unchanged = lines
        .iter()
        .filter(|l| l.starts_with("unchanged: "))
        .count();

    let processes = host.processes();
    let runtime = select(&processes, &plan.canonical_bin_dir, &RUNTIME);
    if runtime.is_empty() {
        println!("stop: no old runtime running from the bin dir");
    } else {
        for (pid, result) in host.terminate(&runtime) {
            let exe = runtime
                .iter()
                .find(|proc| proc.pid == pid)
                .map(|proc| proc.exe.display().to_string())
                .unwrap_or_default();
            match result {
                Ok(()) => {
                    println!("stopped: pid {pid} {exe}");
                    report.stopped.push(pid);
                }
                Err(error) => {
                    println!("could not stop: pid {pid} {exe}: {error}");
                    report.stop_failures.push(format!("pid {pid}: {error}"));
                }
            }
        }
    }
    let old_mcp = select(&processes, &plan.canonical_bin_dir, &[MCP_SERVER])
        .iter()
        .filter(|proc| is_aside(proc))
        .count();
    println!(
        "{old_mcp} {MCP_SERVER} process(es) still run the previous binary; each switches on its client's next session"
    );

    for line in host.remove_stale_asides(&plan.bin_dir) {
        println!("  {line}");
    }

    for client in &CLIENTS {
        let outcome = refresh_client(host, client);
        println!("plugin {}: {outcome}", client.program);
        report
            .plugins
            .push(format!("{}: {outcome}", client.program));
    }
    Ok(report)
}

fn refresh_client(host: &mut dyn Host, client: &Client) -> String {
    if host.find(client.program).is_none() {
        return "not on PATH; skipped".into();
    }
    let list = client.list();
    let Some(output) = host.capture(&list) else {
        return format!("`{list}` failed; skipped");
    };
    if !plugin_installed(&output) {
        return format!("{PLUGIN_ID} is not installed; skipped");
    }
    for cmd in client.refresh_cmds() {
        println!("> {cmd}");
        if let Err(error) = host.run(&cmd) {
            return format!("`{cmd}` failed: {error}");
        }
    }
    "refreshed".into()
}

fn print_summary(report: &Report) {
    let sha = |head: &Option<String>| head.clone().unwrap_or_else(|| "unknown".into());
    println!("summary:");
    println!(
        "  HEAD: {} -> {}",
        sha(&report.head_before),
        sha(&report.head_after)
    );
    println!(
        "  binaries: {} changed, {} unchanged",
        report.changed, report.unchanged
    );
    let pids: Vec<String> = report.stopped.iter().map(u32::to_string).collect();
    println!(
        "  stopped: {} process(es){}",
        pids.len(),
        if pids.is_empty() {
            String::new()
        } else {
            format!(" (pids {})", pids.join(", "))
        }
    );
    for failure in &report.stop_failures {
        println!("  not stopped: {failure}");
    }
    for plugin in &report.plugins {
        println!("  plugin {plugin}");
    }
    println!(
        "  the next Claude Code or Codex hook starts the new runtime (ADR 040); restart Claude Code or start a new session to load plugin changes"
    );
}

/// Locate `name` in `path`, accepting the Windows launcher extensions so npm
/// shims such as `codex.cmd` resolve.
fn find_program(name: &str, path: &[PathBuf]) -> Option<PathBuf> {
    let suffixes: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    path.iter()
        .flat_map(|dir| suffixes.iter().map(move |s| dir.join(format!("{name}{s}"))))
        .find(|candidate| candidate.is_file())
}

/// Performs the real side effects.
struct RealHost {
    path: Vec<PathBuf>,
}

impl RealHost {
    fn command(&self, cmd: &Cmd) -> std::process::Command {
        let program = self
            .find(cmd.program)
            .unwrap_or_else(|| PathBuf::from(cmd.program));
        let mut command = std::process::Command::new(program);
        command.args(&cmd.args);
        if let Some(cwd) = &cmd.cwd {
            command.current_dir(cwd);
        }
        command
    }
}

/// Pids among `pids` that are still running.
fn alive(pids: &[sysinfo::Pid]) -> Vec<sysinfo::Pid> {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(pids),
        true,
        sysinfo::ProcessRefreshKind::new(),
    );
    pids.iter()
        .copied()
        .filter(|pid| system.process(*pid).is_some())
        .collect()
}

fn wait_for_exit(pids: &[sysinfo::Pid], limit: Duration) -> Vec<sysinfo::Pid> {
    let deadline = Instant::now() + limit;
    loop {
        let running = alive(pids);
        if running.is_empty() || Instant::now() >= deadline {
            return running;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

impl Host for RealHost {
    fn find(&self, program: &str) -> Option<PathBuf> {
        find_program(program, &self.path)
    }

    fn run(&mut self, cmd: &Cmd) -> Result<(), String> {
        let status = self
            .command(cmd)
            .status()
            .map_err(|e| format!("cannot start {}: {e}", cmd.program))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("`{cmd}` exited with {status}"))
        }
    }

    fn capture(&mut self, cmd: &Cmd) -> Option<String> {
        let output = self
            .command(cmd)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn processes(&mut self) -> Vec<Proc> {
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
        );
        system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                let exe = process.exe().filter(|exe| !exe.as_os_str().is_empty())?;
                Some(Proc {
                    pid: pid.as_u32(),
                    exe: exe.to_path_buf(),
                })
            })
            .collect()
    }

    fn terminate(&mut self, targets: &[Proc]) -> Vec<(u32, Result<(), String>)> {
        let pids: Vec<sysinfo::Pid> = targets
            .iter()
            .map(|target| sysinfo::Pid::from_u32(target.pid))
            .collect();
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&pids),
            true,
            sysinfo::ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
        );
        for target in targets {
            let Some(process) = system.process(sysinfo::Pid::from_u32(target.pid)) else {
                continue;
            };
            // The pid must still run the selected image; never signal a
            // process that reused it.
            if process.exe() != Some(target.exe.as_path()) {
                continue;
            }
            // Windows has no graceful signal here (`None`); fall back to kill.
            if process.kill_with(sysinfo::Signal::Term) != Some(true) {
                process.kill();
            }
        }
        let mut running = wait_for_exit(&pids, STOP_WAIT);
        if !running.is_empty() {
            for pid in &running {
                if let Some(process) = system.process(*pid) {
                    process.kill();
                }
            }
            running = wait_for_exit(&running, KILL_WAIT);
        }
        targets
            .iter()
            .map(|target| {
                let pid = sysinfo::Pid::from_u32(target.pid);
                let result = if running.contains(&pid) {
                    Err("still running after kill".to_string())
                } else {
                    Ok(())
                };
                (target.pid, result)
            })
            .collect()
    }

    fn install(&mut self, from: &Path, to: &Path) -> Result<Vec<String>, String> {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        install_bins::install(from, to, true, &stamp)
    }

    fn remove_stale_asides(&mut self, dir: &Path) -> Vec<String> {
        install_bins::remove_stale_asides(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every side effect; fails the commands whose program and first
    /// argument match `fail`.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        fail: Vec<(&'static str, &'static str)>,
        install_error: bool,
        processes: Vec<Proc>,
        on_path: Vec<&'static str>,
        plugin_list: String,
    }

    impl Fake {
        fn fails(&self, cmd: &Cmd) -> bool {
            self.fail
                .iter()
                .any(|(program, arg)| cmd.program == *program && cmd.args.iter().any(|a| a == arg))
        }

        fn has(&self, prefix: &str) -> bool {
            self.calls.iter().any(|call| call.starts_with(prefix))
        }
    }

    impl Host for Fake {
        fn find(&self, program: &str) -> Option<PathBuf> {
            self.on_path
                .contains(&program)
                .then(|| PathBuf::from(format!("/usr/bin/{program}")))
        }

        fn run(&mut self, cmd: &Cmd) -> Result<(), String> {
            self.calls.push(format!("run {cmd}"));
            if self.fails(cmd) {
                Err("exit status 1".into())
            } else {
                Ok(())
            }
        }

        fn capture(&mut self, cmd: &Cmd) -> Option<String> {
            self.calls.push(format!("capture {cmd}"));
            if cmd.args.first().map(String::as_str) == Some("plugin") {
                Some(self.plugin_list.clone())
            } else {
                Some("abc1234\n".into())
            }
        }

        fn processes(&mut self) -> Vec<Proc> {
            self.calls.push("processes".into());
            self.processes.clone()
        }

        fn terminate(&mut self, targets: &[Proc]) -> Vec<(u32, Result<(), String>)> {
            let pids: Vec<String> = targets.iter().map(|t| t.pid.to_string()).collect();
            self.calls.push(format!("terminate {}", pids.join(",")));
            targets.iter().map(|t| (t.pid, Ok(()))).collect()
        }

        fn install(&mut self, from: &Path, to: &Path) -> Result<Vec<String>, String> {
            self.calls
                .push(format!("install {} {}", from.display(), to.display()));
            if self.install_error {
                return Err("missing binary".into());
            }
            Ok(vec![
                "installed: cortex-api".into(),
                "unchanged: cortex-hook".into(),
            ])
        }

        fn remove_stale_asides(&mut self, dir: &Path) -> Vec<String> {
            self.calls.push(format!("cleanup {}", dir.display()));
            Vec::new()
        }
    }

    fn workspace(root: &Path) -> PathBuf {
        let source = root.join("checkout");
        std::fs::create_dir_all(source.join("target").join("release")).unwrap();
        std::fs::write(
            source.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        source
    }

    fn proc(pid: u32, exe: PathBuf) -> Proc {
        Proc { pid, exe }
    }

    fn fake_processes(bin: &Path, other: &Path) -> Vec<Proc> {
        vec![
            proc(1, bin.join(install_bins::exe("cortex-api"))),
            proc(
                2,
                bin.join(format!(
                    "{}.old-20261017T101500Z",
                    install_bins::exe("cortex-adapter-claude")
                )),
            ),
            proc(3, bin.join(install_bins::exe("cortex-mcp-server"))),
            proc(4, bin.join(install_bins::exe("cortex-hook"))),
            proc(5, other.join(install_bins::exe("cortex-api"))),
            proc(
                6,
                bin.join(format!(
                    "{}.old-20261017T101500Z",
                    install_bins::exe("cortex-mcp-server")
                )),
            ),
        ]
    }

    #[test]
    fn should_resolve_source_from_manifest_build_dir_by_walking_to_workspace_root() {
        let root = tempfile::tempdir().unwrap();
        let source = workspace(root.path());
        let member = source.join("crates").join("member");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(member.join("Cargo.toml"), "[package]\nname = \"m\"\n").unwrap();
        let build = source.join("target").join("release");
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            bin.join("install-manifest.json"),
            serde_json::json!({"version": 1, "source": build, "binaries": {}}).to_string(),
        )
        .unwrap();

        assert_eq!(resolve_source(None, &bin).unwrap(), source);
        assert_eq!(workspace_root(&member).unwrap(), source);
        assert_eq!(resolve_source(Some(&source), &bin).unwrap(), source);
    }

    #[test]
    fn should_ask_for_source_when_no_manifest_or_no_workspace_is_found() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let error = resolve_source(None, &bin).unwrap_err();
        assert!(error.contains("pass --source"), "{error}");

        // A manifest pointing at a tree with only a package Cargo.toml.
        let lonely = root.path().join("lonely");
        std::fs::create_dir_all(lonely.join("target").join("release")).unwrap();
        std::fs::write(lonely.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            bin.join("install-manifest.json"),
            serde_json::json!({
                "version": 1,
                "source": lonely.join("target").join("release"),
                "binaries": {}
            })
            .to_string(),
        )
        .unwrap();
        let error = resolve_source(None, &bin).unwrap_err();
        assert!(error.contains("pass --source"), "{error}");

        let error = resolve_source(Some(&lonely), &bin).unwrap_err();
        assert!(error.contains("[workspace]"), "{error}");

        // Older installs recorded a relative `--from`; it must not be
        // resolved against whatever directory `update` runs in.
        std::fs::write(
            bin.join("install-manifest.json"),
            serde_json::json!({"version": 1, "source": "target/release", "binaries": {}})
                .to_string(),
        )
        .unwrap();
        let error = resolve_source(None, &bin).unwrap_err();
        assert!(error.contains("relative build dir"), "{error}");
    }

    #[test]
    fn should_use_cargo_target_dir_when_set_and_default_target_otherwise() {
        let source = Path::new("checkout");
        assert_eq!(
            target_dir(source, None),
            source.join("target").join("release")
        );
        assert_eq!(
            target_dir(source, Some(OsString::new())),
            source.join("target").join("release")
        );
        assert_eq!(
            target_dir(source, Some("build-out".into())),
            source.join("build-out").join("release")
        );
        let absolute = std::env::temp_dir().join("cargo-target");
        assert_eq!(
            target_dir(source, Some(absolute.clone().into_os_string())),
            absolute.join("release")
        );
    }

    #[test]
    fn should_select_only_runtime_processes_inside_the_bin_dir() {
        let bin = Path::new("cortex-home").join("bin");
        let other = Path::new("checkout").join("target").join("release");
        let processes = fake_processes(&bin, &other);
        let pids: Vec<u32> = select(&processes, &bin, &RUNTIME)
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(pids, vec![1, 2]);
        let mcp: Vec<u32> = select(&processes, &bin, &[MCP_SERVER])
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(mcp, vec![3, 6]);

        // A subdirectory of the bin dir and a look-alike name are not ours.
        let nested = vec![
            proc(7, bin.join("nested").join(install_bins::exe("cortex-api"))),
            proc(8, bin.join(install_bins::exe("cortex-apiary"))),
        ];
        assert!(select(&nested, &bin, &RUNTIME).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn should_match_windows_paths_case_insensitively_and_ignore_verbatim_prefix() {
        let bin = PathBuf::from(r"\\?\C:\Users\Op\.cortex\bin");
        let processes = vec![
            proc(1, PathBuf::from(r"c:\users\op\.CORTEX\BIN\Cortex-Api.EXE")),
            proc(
                2,
                PathBuf::from(r"C:/Users/Op/.cortex/bin/cortex-adapter-claude.exe.old-s1"),
            ),
            proc(3, PathBuf::from(r"C:\Users\Op\.cortex\bin2\cortex-api.exe")),
        ];
        let pids: Vec<u32> = select(&processes, &bin, &RUNTIME)
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(pids, vec![1, 2]);
    }

    #[test]
    fn should_detect_installed_plugin_in_claude_and_codex_list_output() {
        let claude = "Installed plugins:\n\n  ❯ cortex@hivellm-cortex\n    Version: 0.2.0\n";
        assert!(plugin_installed(claude));
        let codex = "PLUGIN                 STATUS              VERSION  SOURCE\ncortex@hivellm-cortex  installed, enabled  0.2.0    E:\\Cortex\\packages\\cortex-codex-plugin\n";
        assert!(plugin_installed(codex));
        let available = "cortex@hivellm-cortex  not installed                E:\\Cortex\\packages\\cortex-codex-plugin\n";
        assert!(!plugin_installed(available));
        let lookalike = "snowflake-cortex-code@claude-plugins-official  installed, enabled\n  ❯ cortex@other-market\n";
        assert!(!plugin_installed(lookalike));
        assert!(!plugin_installed(""));
    }

    fn run_apply(fake: &mut Fake, no_pull: bool) -> Result<Option<Report>, String> {
        let root = tempfile::tempdir().unwrap();
        let source = workspace(root.path());
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let bin = bin.canonicalize().unwrap();
        if fake.processes.is_empty() {
            fake.processes = fake_processes(&bin, &source.join("target").join("release"));
        }
        execute(fake, &bin, Some(&source), no_pull, true, None)
    }

    #[test]
    fn should_not_build_install_or_stop_anything_when_git_pull_fails() {
        let mut fake = Fake {
            fail: vec![("git", "pull")],
            ..Fake::default()
        };
        let error = run_apply(&mut fake, false).unwrap_err();
        assert!(error.contains("git pull failed"), "{error}");
        assert!(fake.has("run git -C "), "{:?}", fake.calls);
        for step in ["run cargo", "install", "processes", "terminate", "cleanup"] {
            assert!(!fake.has(step), "{step} ran: {:?}", fake.calls);
        }
    }

    #[test]
    fn should_not_install_or_stop_anything_when_build_fails() {
        let mut fake = Fake {
            fail: vec![("cargo", "build")],
            ..Fake::default()
        };
        let error = run_apply(&mut fake, false).unwrap_err();
        assert!(error.contains("build failed"), "{error}");
        assert!(fake.has("run cargo build --release"), "{:?}", fake.calls);
        for step in ["install", "processes", "terminate", "cleanup"] {
            assert!(!fake.has(step), "{step} ran: {:?}", fake.calls);
        }
    }

    #[test]
    fn should_not_stop_anything_when_install_fails() {
        let mut fake = Fake {
            install_error: true,
            ..Fake::default()
        };
        let error = run_apply(&mut fake, true).unwrap_err();
        assert!(error.contains("install failed"), "{error}");
        for step in ["processes", "terminate", "cleanup"] {
            assert!(!fake.has(step), "{step} ran: {:?}", fake.calls);
        }
    }

    #[test]
    fn should_run_steps_in_order_and_stop_only_bin_dir_runtime() {
        let mut fake = Fake {
            on_path: vec!["claude", "codex"],
            plugin_list: "cortex@hivellm-cortex  installed, enabled  0.2.0\n".into(),
            fail: vec![("codex", "add")],
            ..Fake::default()
        };
        let report = run_apply(&mut fake, false).unwrap().unwrap();
        let kinds: Vec<&str> = fake
            .calls
            .iter()
            .map(|call| {
                let words: Vec<&str> = call.split_whitespace().collect();
                match words.as_slice() {
                    ["capture", "git", _, _, "rev-parse", ..] => "head",
                    ["run", "git", _, _, "pull", "--ff-only"] => "pull",
                    ["run", "cargo", "build", "--release", ..] => "build",
                    ["install", ..] => "install",
                    ["processes"] => "processes",
                    ["terminate", "1,2"] => "terminate",
                    ["cleanup", ..] => "cleanup",
                    ["capture", "claude", "plugin", "list"] => "claude-list",
                    ["run", "claude", "plugin", "marketplace", "update", "hivellm-cortex"] => {
                        "claude-marketplace"
                    }
                    ["run", "claude", "plugin", "update", "cortex@hivellm-cortex"] => {
                        "claude-update"
                    }
                    ["capture", "codex", "plugin", "list"] => "codex-list",
                    ["run", "codex", "plugin", "add", "cortex@hivellm-cortex"] => "codex-add",
                    _ => "unexpected",
                }
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "head",
                "pull",
                "build",
                "head",
                "install",
                "processes",
                "terminate",
                "cleanup",
                "claude-list",
                "claude-marketplace",
                "claude-update",
                "codex-list",
                "codex-add",
            ],
            "{:?}",
            fake.calls
        );
        let build = fake.calls.iter().find(|c| c.contains("cargo")).unwrap();
        for name in BINARIES {
            assert!(build.contains(&format!("--bin {name}")), "{build}");
        }
        assert_eq!(report.stopped, vec![1, 2]);
        assert_eq!((report.changed, report.unchanged), (1, 1));
        assert_eq!(report.head_before.as_deref(), Some("abc1234"));
        assert_eq!(report.plugins[0], "claude: refreshed");
        assert!(report.plugins[1].starts_with("codex: `codex plugin add"));
    }

    #[test]
    fn should_skip_pull_and_plugins_that_are_absent_or_not_installed() {
        let mut fake = Fake {
            on_path: vec!["codex"],
            plugin_list: "cortex@hivellm-cortex  not installed\n".into(),
            ..Fake::default()
        };
        let report = run_apply(&mut fake, true).unwrap().unwrap();
        assert!(!fake.has("run git"), "{:?}", fake.calls);
        assert!(!fake.has("run codex"), "{:?}", fake.calls);
        assert_eq!(report.plugins[0], "claude: not on PATH; skipped");
        assert!(report.plugins[1].contains("not installed"));
    }

    #[test]
    fn should_only_read_state_during_dry_run() {
        let root = tempfile::tempdir().unwrap();
        let source = workspace(root.path());
        let bin = root.path().join("bin");
        let mut fake = Fake {
            on_path: vec!["claude", "codex"],
            processes: fake_processes(&bin, &source),
            ..Fake::default()
        };
        assert!(execute(&mut fake, &bin, Some(&source), false, false, None)
            .unwrap()
            .is_none());
        for step in ["run ", "install", "terminate", "cleanup", "capture claude"] {
            assert!(!fake.has(step), "{step} ran: {:?}", fake.calls);
        }
        assert!(!bin.exists());
    }

    #[test]
    fn should_display_commands_as_run() {
        let source = Path::new("my checkout");
        assert_eq!(
            git(source, &["pull", "--ff-only"]).to_string(),
            "git -C \"my checkout\" pull --ff-only"
        );
        assert_eq!(
            build_cmd(source).to_string(),
            "cargo build --release --bin cortex-api --bin cortex-adapter-claude --bin cortex-hook --bin cortex-mcp-server --bin cortex (in my checkout)"
        );
    }

    #[test]
    fn should_find_programs_on_path_including_windows_launchers() {
        let root = tempfile::tempdir().unwrap();
        let empty = root.path().join("empty");
        let tools = root.path().join("tools");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&tools).unwrap();
        let launcher = if cfg!(windows) { "codex.cmd" } else { "codex" };
        std::fs::write(tools.join(launcher), "").unwrap();
        let path = vec![empty.clone(), tools.clone()];
        assert_eq!(find_program("codex", &path), Some(tools.join(launcher)));
        assert_eq!(find_program("claude", &path), None);
    }
}
