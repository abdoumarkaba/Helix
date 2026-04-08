#![allow(clippy::pedantic)]
use std::path::PathBuf;

use crate::models::errors::PlayError;
use crate::models::plan::{DbEntry, RunnersManifest};

// ---------------------------------------------------------------------------
// DatabaseReader trait — injected for testability
// ---------------------------------------------------------------------------

/// Abstraction over the local play-db cache.
/// Tests inject `FixtureReader`; production uses `DatabaseClient`.
pub trait DatabaseReader {
    fn lookup(&self, exe_hash: &str) -> Result<Option<DbEntry>, PlayError>;
    fn load_runners_manifest(&self) -> Result<RunnersManifest, PlayError>;
}

// ---------------------------------------------------------------------------
// DatabaseClient — reads from ~/.local/share/play/db/
// ---------------------------------------------------------------------------

pub struct DatabaseClient {
    db_root: PathBuf,
}

impl DatabaseClient {
    pub fn new(db_root: PathBuf) -> Self {
        Self { db_root }
    }
}

impl DatabaseReader for DatabaseClient {
    fn lookup(&self, exe_hash: &str) -> Result<Option<DbEntry>, PlayError> {
        if exe_hash.len() < 2 {
            return Ok(None);
        }
        let prefix = &exe_hash[..2];
        let entry_path = self
            .db_root
            .join("entries")
            .join(prefix)
            .join(exe_hash)
            .join("default.toml");
        if !entry_path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&entry_path).map_err(|e| PlayError::PlanningFailed {
            reason: format!("Failed to read DB entry at {}: {e}", entry_path.display()),
        })?;
        let entry: DbEntry = toml::from_str(&raw).map_err(|e| PlayError::DatabaseCorrupted {
            hash: exe_hash.to_owned(),
            reason: e.to_string(),
        })?;
        Ok(Some(entry))
    }

    fn load_runners_manifest(&self) -> Result<RunnersManifest, PlayError> {
        let path = self.db_root.join("runners.toml");
        if !path.exists() {
            return Err(PlayError::RunnersManifestMissing { path });
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| PlayError::PlanningFailed {
            reason: format!("Failed to read runners.toml: {e}"),
        })?;
        toml::from_str(&raw).map_err(|e| PlayError::PlanningFailed {
            reason: format!("runners.toml parse error: {e}"),
        })
    }
}

// ---------------------------------------------------------------------------
// NoopDatabaseReader — for tests with no DB entry
// ---------------------------------------------------------------------------

/// Always returns None (heuristics only). Used in unit tests.
pub struct NoopDatabaseReader {
    manifest: RunnersManifest,
}

impl NoopDatabaseReader {
    pub fn with_manifest(manifest: RunnersManifest) -> Self {
        Self { manifest }
    }
}

impl DatabaseReader for NoopDatabaseReader {
    fn lookup(&self, _exe_hash: &str) -> Result<Option<DbEntry>, PlayError> {
        Ok(None)
    }
    fn load_runners_manifest(&self) -> Result<RunnersManifest, PlayError> {
        Ok(self.manifest.clone())
    }
}

// ---------------------------------------------------------------------------
// FixtureReader — for integration tests
// ---------------------------------------------------------------------------

/// Reads from a test fixture directory.
pub struct FixtureReader {
    fixture_root: PathBuf,
}

impl FixtureReader {
    pub fn new(fixture_root: impl Into<PathBuf>) -> Self {
        Self {
            fixture_root: fixture_root.into(),
        }
    }
}

impl DatabaseReader for FixtureReader {
    fn lookup(&self, exe_hash: &str) -> Result<Option<DbEntry>, PlayError> {
        if exe_hash.len() < 2 {
            return Ok(None);
        }
        let prefix = &exe_hash[..2];
        let path = self
            .fixture_root
            .join("entries")
            .join(prefix)
            .join(exe_hash)
            .join("default.toml");
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| PlayError::PlanningFailed {
            reason: format!("fixture read error: {e}"),
        })?;
        toml::from_str(&raw)
            .map_err(|e| PlayError::DatabaseCorrupted {
                hash: exe_hash.to_owned(),
                reason: e.to_string(),
            })
            .map(Some)
    }

    fn load_runners_manifest(&self) -> Result<RunnersManifest, PlayError> {
        let path = self.fixture_root.join("runners.toml");
        if !path.exists() {
            return Err(PlayError::RunnersManifestMissing { path });
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| PlayError::PlanningFailed {
            reason: format!("fixture runners.toml read error: {e}"),
        })?;
        toml::from_str(&raw).map_err(|e| PlayError::PlanningFailed {
            reason: format!("fixture runners.toml parse error: {e}"),
        })
    }
}
