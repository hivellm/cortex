//! Explicit, source-preserving migration into the local canonical store.
pub mod archive;
pub mod daily;

use crate::{files, LocalError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use archive::import_archive;
pub use daily::import_daily;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOptions {
    pub namespace: String,
    #[serde(default)]
    pub repository_map: BTreeMap<String, String>,
    pub unscoped_repository: Option<String>,
    pub global_repository: Option<String>,
    /// Maximum archive files needing decoding. Verifying unchanged files is separate.
    pub max_files: usize,
    /// Maximum source events or daily revisions processed in this invocation.
    pub max_records: usize,
    pub dry_run: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            namespace: "legacy".into(),
            repository_map: BTreeMap::new(),
            unscoped_repository: None,
            global_repository: None,
            max_files: 100_000,
            max_records: 10_000,
            dry_run: false,
        }
    }
}

impl ImportOptions {
    pub(crate) fn validate(&self) -> Result<()> {
        files::validate_id(&self.namespace)?;
        if !(1..=1_000_000).contains(&self.max_files)
            || !(1..=1_000_000).contains(&self.max_records)
        {
            return Err(LocalError::Invalid(
                "import limits must be 1..1000000".into(),
            ));
        }
        for (source, target) in &self.repository_map {
            files::validate_scope(source)?;
            files::validate_scope(target)?;
        }
        for target in [&self.unscoped_repository, &self.global_repository]
            .into_iter()
            .flatten()
        {
            files::validate_scope(target)?;
        }
        Ok(())
    }

    pub(crate) fn repository(&self, source: Option<&str>) -> Result<String> {
        match source {
            Some(source) => {
                files::validate_scope(source)?;
                Ok(self
                    .repository_map
                    .get(source)
                    .map_or(source, String::as_str)
                    .to_owned())
            }
            None => self.unscoped_repository.clone().ok_or_else(|| {
                LocalError::Invalid(
                    "source has no repository; supply an explicit unscoped repository mapping"
                        .into(),
                )
            }),
        }
    }

    pub(crate) fn mapping_hash(&self) -> Result<String> {
        Ok(files::hash(&files::canonical(&(
            &self.namespace,
            &self.repository_map,
            &self.unscoped_repository,
            &self.global_repository,
        ))?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportFailure {
    pub source: String,
    pub position: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub source: String,
    /// Includes unchanged files whose compressed source fingerprint was verified.
    pub files_scanned: usize,
    pub unchanged_files: usize,
    pub records_seen: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub rejected: usize,
    pub quarantined_files: usize,
    /// Excluded compressed event archives (never decoded).
    #[serde(default)]
    pub quarantined_archives: usize,
    /// Excluded reason sidecars, corrupt remnants, or other non-archive entries.
    #[serde(default)]
    pub quarantine_diagnostics: usize,
    pub incomplete_files: usize,
    /// EOF inside an unfinished frame, with every decoded line already committed.
    #[serde(default)]
    pub open_frame_files: usize,
    /// EOF with decoded bytes of an unterminated, uncommitted line.
    #[serde(default)]
    pub partial_line_files: usize,
    #[serde(default)]
    pub partial_line_bytes: u64,
    /// Decoder failures block the remaining file; they are not decoded-record rejections.
    #[serde(default)]
    pub decoder_failed_files: usize,
    /// Subset of decoder failures caused by a bounded line ceiling.
    #[serde(default)]
    pub line_limit_files: usize,
    pub bytes_scanned: u64,
    pub limit_reached: bool,
    pub complete: bool,
    pub errors: Vec<ImportFailure>,
}

impl ImportReport {
    pub(crate) fn failure(
        &mut self,
        source: &str,
        position: Option<String>,
        reason: impl ToString,
    ) {
        if self.errors.len() < 100 {
            self.errors.push(ImportFailure {
                source: source.into(),
                position,
                reason: reason.to_string(),
            });
        }
    }
}

pub(crate) fn identity(kind: &str, namespace: &str, scope: &str, id: &str) -> Result<String> {
    Ok(format!(
        "{kind}-{}",
        files::hash(&files::canonical(&(namespace, scope, id))?)
    ))
}
