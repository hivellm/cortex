//! Explicit offline updates run the candidate's installer so assets match its version.
use crate::{files, install::InstallReport, LocalError, Result};
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_OUTPUT: u64 = 128 * 1024;

fn invoke(binary: &Path, args: &[&OsStr], timeout: Duration) -> Result<Value> {
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?)
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        let poll = (|| -> Result<_> {
            if stdout.metadata()?.len() > MAX_OUTPUT || stderr.metadata()?.len() > MAX_OUTPUT {
                return Err(LocalError::Invalid(
                    "update subprocess output exceeds limit".into(),
                ));
            }
            if started.elapsed() > timeout {
                return Err(LocalError::Invalid("update subprocess timed out".into()));
            }
            Ok(child.try_wait()?)
        })();
        match poll {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
    };
    if !status.success() {
        stderr.seek(SeekFrom::Start(0))?;
        let mut message = String::new();
        stderr.take(MAX_OUTPUT).read_to_string(&mut message)?;
        return Err(LocalError::Invalid(format!(
            "update subprocess failed: {}",
            message.trim()
        )));
    }
    stdout.seek(SeekFrom::Start(0))?;
    Ok(serde_json::from_reader(stdout.take(MAX_OUTPUT + 1))?)
}

pub fn update(home: &Path, binary: &Path) -> Result<InstallReport> {
    let binary = files::existing_regular_file(binary)?;
    let metadata = fs::symlink_metadata(&binary)?;
    if !metadata.is_file() || metadata.len() > 512 * 1024 * 1024 {
        return Err(LocalError::Invalid(
            "update requires a regular standalone executable under 512 MiB".into(),
        ));
    }
    let info = invoke(&binary, &[OsStr::new("build-info")], Duration::from_secs(5))?;
    if info["runtime"] != "standalone" || info["install_protocol"] != 1 {
        return Err(LocalError::Invalid(
            "candidate does not support standalone install protocol 1".into(),
        ));
    }
    let report = invoke(
        &binary,
        &[
            OsStr::new("--home"),
            home.as_os_str(),
            OsStr::new("install"),
        ],
        Duration::from_secs(60),
    )?;
    Ok(serde_json::from_value(report)?)
}
