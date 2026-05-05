/// Shared path-resolution helpers used across `cortex-ops` submodules.

/// Phase11s §2.4 — match the worker's metadata-DB resolution
/// precedence so `cortex-ops graph replay` writes to the same row
/// the worker reads on boot.
pub(super) fn resolve_metadata_db_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("CORTEX_GRAPH_METADATA_DB") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CORTEX_METADATA_DB") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("CORTEX_HOME") {
        return std::path::PathBuf::from(home).join("metadata.sqlite");
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".cortex").join("metadata.sqlite")
}

pub(super) fn resolve_metadata_db(arg: Option<String>) -> Option<std::path::PathBuf> {
    if let Some(p) = arg {
        return Some(std::path::PathBuf::from(p));
    }
    if let Ok(p) = std::env::var("CORTEX_METADATA_DB") {
        if !p.is_empty() {
            return Some(std::path::PathBuf::from(p));
        }
    }
    // phase11x — honour `CORTEX_HOME` so `cortex-ops` sees the same
    // metadata DB the daemon writes to. Inside the docker stack the
    // daemon runs with `CORTEX_HOME=/var/lib/cortex`; without this
    // fallback `cortex-ops schedule list` resolves to
    // `<HOME>/.cortex/metadata.sqlite` (HOME default) and prints an
    // empty registry instead of the real one.
    if let Ok(home) = std::env::var("CORTEX_HOME") {
        if !home.is_empty() {
            return Some(std::path::PathBuf::from(home).join("metadata.sqlite"));
        }
    }
    if let Some(home) = home_dir() {
        return Some(home.join(".cortex").join("metadata.sqlite"));
    }
    None
}

pub(super) fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
}
