#![allow(clippy::pedantic)]
/// PlanningModule - entry point for the planning phase.
///
/// Receives a GameEnvironment (hardware + identity fields populated by DetectionModule)
/// and produces a fully-resolved GamePlan ready for user confirmation.
/// No system state is written here.
use std::path::PathBuf;

use crate::models::environment::GameEnvironment;
use crate::models::errors::PlayError;
use crate::models::plan::GamePlan;

use super::database_client::DatabaseReader;
use super::plan_builder::PlanBuilder;
use super::runner_manifest;

pub struct PlanningModule {
    /// Root directory where runner binaries are installed.
    runners_install_root: PathBuf,
    /// Root directory for Wine prefixes.
    prefix_root: PathBuf,
    /// Optional play-db reader.
    db_reader: Option<Box<dyn DatabaseReader>>,
}

impl PlanningModule {
    pub fn new(
        runners_install_root: PathBuf,
        prefix_root: PathBuf,
        db_reader: Option<Box<dyn DatabaseReader>>,
    ) -> Self {
        Self { runners_install_root, prefix_root, db_reader }
    }

    /// Plan execution for the given game environment.
    pub fn plan(&self, env: &GameEnvironment) -> Result<GamePlan, PlayError> {
        let (manifest, db_entry) = if let Some(reader) = &self.db_reader {
            let manifest = reader
                .load_runners_manifest()
                .unwrap_or_else(|_| runner_manifest::get_default_manifest());
            let entry = reader.lookup(&env.identity.exe_hash).unwrap_or(None);
            (manifest, entry)
        } else {
            (runner_manifest::get_default_manifest(), None)
        };

        PlanBuilder::new(
            env,
            &manifest,
            self.prefix_root.clone(),
            self.runners_install_root.clone(),
            db_entry,
        )
        .build()
    }
}
