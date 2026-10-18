//! Immutable canonical publication. Paths are hashes, never caller filenames.
use crate::model::*;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn canonical<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    cortex_core::canonical_json::canonicalize(&serde_json::to_value(value)?)
        .map_err(|e| LocalError::Invalid(e.to_string()))
}

pub(crate) fn seal(record: &mut Record) -> Result<()> {
    record.content_sha256.clear();
    record.content_sha256 = hash(&canonical(record)?);
    validate(record)
}

pub(crate) fn validate_scope(repository: &str) -> Result<()> {
    if repository.trim().is_empty()
        || repository.len() > 1024
        || repository.chars().any(char::is_control)
    {
        return Err(LocalError::Invalid(
            "repository must be explicit, nonempty and bounded".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    if id.trim().is_empty() || id.len() > 1024 || id.chars().any(char::is_control) {
        return Err(LocalError::Invalid(
            "record ID must be nonempty and bounded".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate(record: &Record) -> Result<()> {
    validate_scope(&record.repository)?;
    validate_id(&record.id)?;
    if record.schema_version != 1 || record.revision == 0 || record.revision > i64::MAX as u64 {
        return Err(LocalError::Invalid("unsupported schema or revision".into()));
    }
    if record.body.len() > MAX_BODY_BYTES || record.title.len() > 16 * 1024 {
        return Err(LocalError::Invalid(
            "body exceeds 1 MiB or title exceeds 16 KiB".into(),
        ));
    }
    if record.links.len() > 128 {
        return Err(LocalError::Invalid("at most 128 links per record".into()));
    }
    for link in &record.links {
        validate_id(&link.target_id)?;
    }
    chrono::DateTime::parse_from_rfc3339(&record.created_at)
        .map_err(|e| LocalError::Invalid(format!("created_at: {e}")))?;
    chrono::DateTime::parse_from_rfc3339(&record.updated_at)
        .map_err(|e| LocalError::Invalid(format!("updated_at: {e}")))?;
    let bytes = canonical(record)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(LocalError::Invalid("canonical record exceeds 4 MiB".into()));
    }
    let mut unsigned = record.clone();
    unsigned.content_sha256.clear();
    if hash(&canonical(&unsigned)?) != record.content_sha256 {
        return Err(LocalError::Invalid(format!(
            "content hash mismatch: {}",
            record.id
        )));
    }
    if record.kind == RecordKind::Event
        && (record.revision != 1 || record.authority != Authority::Observation)
    {
        return Err(LocalError::Invalid(
            "raw events must remain immutable observations".into(),
        ));
    }
    Ok(())
}

fn is_redirect(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// Resolve legitimate home-directory aliases, but never accept a redirected
/// store root itself. Subsequent store paths use this canonical root.
pub(crate) fn prepare_root(path: &Path) -> Result<PathBuf> {
    // Remove a trailing separator or `.` before lstat; otherwise some systems
    // dereference a symlink at the final component before inspecting it.
    let path: PathBuf = path.components().collect();
    let reject_redirect = || -> Result<()> {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if is_redirect(&metadata) => Err(LocalError::Invalid(format!(
                "redirected store root forbidden: {}",
                path.display()
            ))),
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    };
    reject_redirect()?;
    fs::create_dir_all(&path)?;
    reject_redirect()?;
    Ok(fs::canonicalize(&path)?)
}

/// Resolve an existing root without creating directories. Ancestor aliases are
/// legitimate home paths; redirecting the store root itself remains forbidden.
pub(crate) fn existing_root(path: &Path) -> Result<PathBuf> {
    let path: PathBuf = path.components().collect();
    if is_redirect(&fs::symlink_metadata(&path)?) {
        return Err(LocalError::Invalid(
            "redirected store root forbidden".into(),
        ));
    }
    let resolved = fs::canonicalize(&path)?;
    ensure_no_symlinks(&resolved)?;
    if !resolved.is_dir() {
        return Err(LocalError::Invalid("store root must be a directory".into()));
    }
    Ok(resolved)
}

/// Resolve ancestor aliases without accepting a redirected executable itself.
pub(crate) fn existing_regular_file(path: &Path) -> Result<PathBuf> {
    let path: PathBuf = path.components().collect();
    let metadata = fs::symlink_metadata(&path)?;
    if is_redirect(&metadata) || !metadata.is_file() {
        return Err(LocalError::Invalid(
            "expected a regular file without redirection".into(),
        ));
    }
    let resolved = fs::canonicalize(&path)?;
    ensure_no_symlinks(&resolved)?;
    Ok(resolved)
}

pub(crate) fn ensure_no_symlinks(path: &Path) -> Result<()> {
    let mut part = PathBuf::new();
    for component in path.components() {
        part.push(component);
        // A Windows verbatim drive prefix (\\?\C:) is not itself a usable
        // filesystem path until its root separator has also been appended.
        if matches!(component, std::path::Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&part) {
            Ok(meta) if is_redirect(&meta) => {
                return Err(LocalError::Invalid(format!(
                    "symlink forbidden: {}",
                    part.display()
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub(crate) fn memory_path(root: &Path, record: &Record) -> PathBuf {
    root.join("records")
        .join(hash(record.repository.as_bytes()))
        .join(hash(record.id.as_bytes()))
        .join(format!("{:020}.md", record.revision))
}

pub(crate) fn publish_memory(root: &Path, record: &Record) -> Result<()> {
    let mut header = record.clone();
    header.body.clear();
    let mut bytes = b"---json\n".to_vec();
    bytes.extend(canonical(&header)?);
    bytes.extend(b"\n---\n");
    bytes.extend(record.body.as_bytes());
    publish(root, &memory_path(root, record), &bytes)
}

pub(crate) fn publish_segment(root: &Path, records: &[Record]) -> Result<()> {
    let mut bytes = Vec::new();
    for record in records {
        bytes.extend(canonical(record)?);
        bytes.push(b'\n');
    }
    let path = root
        .join("segments")
        .join(format!("{}.jsonl", hash(&bytes)));
    publish(root, &path, &bytes)
}

pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) fn publish(root: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    if !path.starts_with(root) || path == root {
        return Err(LocalError::Invalid(
            "publication lies outside store root".into(),
        ));
    }
    ensure_no_symlinks(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| LocalError::Invalid("missing parent".into()))?;
    fs::create_dir_all(parent)?;
    ensure_no_symlinks(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".cortex-")
        .tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(path) {
        Ok(published) => {
            published.sync_all()?;
            for directory in parent.ancestors() {
                sync_directory(directory)?;
                if directory == root {
                    break;
                }
            }
            Ok(())
        }
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.is_file() || metadata.len() != bytes.len() as u64 {
                return Err(LocalError::Conflict(format!(
                    "immutable file already exists with different size or type: {}",
                    path.display()
                )));
            }
            let mut existing = Vec::with_capacity(bytes.len());
            File::open(path)?
                .take(bytes.len() as u64 + 1)
                .read_to_end(&mut existing)?;
            if existing == bytes {
                Ok(())
            } else {
                Err(LocalError::Conflict(format!(
                    "immutable file already exists: {}",
                    path.display()
                )))
            }
        }
        Err(e) => Err(e.error.into()),
    }
}

pub(crate) fn visit_records(
    root: &Path,
    mut visit: impl FnMut(Record) -> Result<()>,
) -> Result<usize> {
    let mut count = 0;
    for subtree in ["records", "segments"] {
        let base = root.join(subtree);
        ensure_no_symlinks(&base)?;
        if !base.exists() {
            continue;
        }
        let mut pending = vec![base];
        while let Some(directory) = pending.pop() {
            ensure_no_symlinks(&directory)?;
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    return Err(LocalError::Invalid(format!(
                        "symlink in canonical store: {}",
                        path.display()
                    )));
                }
                if kind.is_dir() {
                    pending.push(path);
                    continue;
                }
                if !kind.is_file() {
                    return Err(LocalError::Invalid(format!(
                        "non-regular canonical file: {}",
                        path.display()
                    )));
                }
                if entry.file_name().to_string_lossy().starts_with(".cortex-") {
                    continue;
                }
                count += 1;
                if subtree == "records" && path.extension().is_some_and(|e| e == "md") {
                    if entry.metadata()?.len() > (MAX_RECORD_BYTES + 32) as u64 {
                        return Err(LocalError::Invalid(
                            "oversized canonical memory file".into(),
                        ));
                    }
                    let mut bytes = Vec::new();
                    File::open(&path)?
                        .take((MAX_RECORD_BYTES + 32) as u64)
                        .read_to_end(&mut bytes)?;
                    let text = std::str::from_utf8(&bytes)
                        .map_err(|e| LocalError::Invalid(e.to_string()))?;
                    let rest = text.strip_prefix("---json\n").ok_or_else(|| {
                        LocalError::Invalid(format!("invalid frontmatter: {}", path.display()))
                    })?;
                    let (header, body) = rest
                        .split_once("\n---\n")
                        .ok_or_else(|| LocalError::Invalid("missing frontmatter end".into()))?;
                    let mut record: Record = serde_json::from_str(header)?;
                    if !record.body.is_empty() || record.kind == RecordKind::Event {
                        return Err(LocalError::Invalid("invalid memory frontmatter".into()));
                    }
                    record.body = body.to_owned();
                    validate(&record)?;
                    if memory_path(root, &record) != path {
                        return Err(LocalError::Invalid("canonical memory path mismatch".into()));
                    }
                    visit(record)?;
                } else if subtree == "segments" && path.extension().is_some_and(|e| e == "jsonl") {
                    if entry.metadata()?.len() > MAX_BATCH_BYTES as u64 {
                        return Err(LocalError::Invalid("segment exceeds 8 MiB".into()));
                    }
                    let mut reader = BufReader::new(File::open(&path)?);
                    let mut digest = Sha256::new();
                    let mut records = 0;
                    loop {
                        let mut line = Vec::new();
                        (&mut reader)
                            .take((MAX_RECORD_BYTES + 2) as u64)
                            .read_until(b'\n', &mut line)?;
                        if line.is_empty() {
                            break;
                        }
                        if line.len() > MAX_RECORD_BYTES + 1 || line.last() != Some(&b'\n') {
                            return Err(LocalError::Invalid(
                                "oversized or incomplete segment record".into(),
                            ));
                        }
                        records += 1;
                        if records > MAX_BATCH_RECORDS {
                            return Err(LocalError::Invalid("segment exceeds 256 records".into()));
                        }
                        digest.update(&line);
                        let record: Record = serde_json::from_slice(&line)?;
                        validate(&record)?;
                        if record.kind != RecordKind::Event {
                            return Err(LocalError::Invalid(
                                "segment contains non-event record".into(),
                            ));
                        }
                        visit(record)?;
                    }
                    if records == 0
                        || path.file_stem().and_then(|s| s.to_str())
                            != Some(format!("{:x}", digest.finalize()).as_str())
                    {
                        return Err(LocalError::Invalid("segment filename hash mismatch".into()));
                    }
                } else {
                    return Err(LocalError::Invalid(format!(
                        "unexpected canonical file: {}",
                        path.display()
                    )));
                }
            }
        }
    }
    Ok(count)
}
