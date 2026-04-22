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

use super::plan_builder::PlanBuilder;
use super::runner_manifest;

pub struct PlanningModule {
    /// Root directory where runner binaries are installed.
    runners_install_root: PathBuf,
    /// Root directory for Wine prefixes (e.g. ~/.local/share/play/prefixes).
    prefix_root: PathBuf,
}

impl PlanningModule {
    pub fn new(runners_install_root: PathBuf, prefix_root: PathBuf) -> Self {
        Self { runners_install_root, prefix_root }
    }

    /// Plan execution for the given game environment.
    pub fn plan(&self, env: &GameEnvironment) -> Result<GamePlan, PlayError> {
        let manifest = runner_manifest::get_default_manifest();
        PlanBuilder::new(env, &manifest, self.prefix_root.clone(), self.runners_install_root.clone())
            .build()
    }
}
