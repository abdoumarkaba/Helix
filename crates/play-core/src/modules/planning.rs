#![allow(clippy::pedantic)]
/// PlanningModule — entry point for the planning phase.
///
/// Receives a GameEnvironment (hardware + identity fields populated by DetectionModule)
/// and produces a fully-resolved GamePlan ready for user confirmation.
/// No system state is written here.
use std::path::PathBuf;

use crate::models::environment::GameEnvironment;
use crate::models::errors::PlayError;
use crate::models::plan::GamePlan;

use super::database_client::{DatabaseClient, DatabaseReader};
use super::plan_builder::PlanBuilder;

pub struct PlanningModule {
    /// Root of the local play-db cache (e.g. ~/.local/share/play/db).
    db_root: PathBuf,
    /// Root directory where runner binaries are installed.
    runners_install_root: PathBuf,
    /// Root directory for Wine prefixes (e.g. ~/.local/share/play/prefixes).
    prefix_root: PathBuf,
}

impl PlanningModule {
    pub fn new(db_root: PathBuf, runners_install_root: PathBuf, prefix_root: PathBuf) -> Self {
        Self { db_root, runners_install_root, prefix_root }
    }

    /// Plan execution for the given game environment.
    /// Returns a GamePlan with `hard_blocks` populated if planning fails.
    pub fn plan(&self, env: &GameEnvironment) -> Result<GamePlan, PlayError> {
        let db = DatabaseClient::new(self.db_root.clone());
        self.plan_with_reader(env, &db)
    }

    /// Plan with an injected DatabaseReader (for testing).
    pub fn plan_with_reader(
        &self,
        env: &GameEnvironment,
        db: &dyn DatabaseReader,
    ) -> Result<GamePlan, PlayError> {
        PlanBuilder::new(env, db, self.prefix_root.clone(), self.runners_install_root.clone())
            .build()
    }
}
